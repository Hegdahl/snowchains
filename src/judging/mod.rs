pub(crate) mod command;
mod interactive;
mod simple;
mod text;

use crate::config::Config;
use crate::errors::{JudgeErrorKind, JudgeResult, TestSuiteResult};
use crate::judging::command::JudgingCommand;
use crate::terminal::{TermOut, WriteAnsi, WriteSpaces};
use crate::testsuite::{SimpleCase, TestCase, TestCases};
use crate::util::std_unstable::AsMillis_;

use futures::{Future, Sink, Stream};
use itertools::Itertools;
use tokio::runtime::Runtime;

use std::io::{self, BufRead};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, fmt};

pub(crate) fn num_cases(config: &Config, problem: &str) -> TestSuiteResult<usize> {
    let (cases, _) = config.testcase_loader().load_merging(problem)?;
    Ok(match cases {
        TestCases::Simple(cases) => cases.len(),
        TestCases::Interactive(cases) => cases.len(),
    })
}

pub(crate) fn timelimit_millis(config: &Config, problem: &str, nth: usize) -> JudgeResult<u128> {
    fn get_timelimit_millis<C>(
        cases: &[C],
        nth: usize,
        f: fn(&C) -> Option<Duration>,
    ) -> JudgeResult<u128> {
        cases
            .get(nth)
            .and_then(f)
            .map(AsMillis_::as_millis_)
            .ok_or_else(|| JudgeErrorKind::IndexOutOfBounds(cases.len(), nth).into())
    }

    let (cases, _) = config.testcase_loader().load_merging(problem)?;
    match cases {
        TestCases::Simple(cases) => get_timelimit_millis(&cases, nth, |t| t.timelimit()),
        TestCases::Interactive(cases) => get_timelimit_millis(&cases, nth, |t| t.timelimit()),
    }
}

pub(crate) fn input(config: &Config, problem: &str, nth: usize) -> JudgeResult<Arc<String>> {
    let (cases, _) = config.testcase_loader().load_merging(problem)?;
    match &cases {
        TestCases::Simple(cases) => cases
            .get(nth)
            .map(SimpleCase::input)
            .ok_or_else(|| JudgeErrorKind::IndexOutOfBounds(cases.len(), nth).into()),
        TestCases::Interactive(cases) if nth < cases.len() => Ok(Arc::new("".to_owned())),
        TestCases::Interactive(cases) => {
            Err(JudgeErrorKind::IndexOutOfBounds(cases.len(), nth).into())
        }
    }
}

pub(crate) fn accepts(
    config: &Config,
    problem: &str,
    nth: usize,
    mut stdin: impl BufRead,
    mut stderr: impl TermOut,
) -> JudgeResult<()> {
    let (cases, _) = config.testcase_loader().load_merging(problem)?;
    match cases {
        TestCases::Simple(cases) => {
            let case = cases
                .get(nth)
                .ok_or_else(|| JudgeErrorKind::IndexOutOfBounds(cases.len(), nth))?;
            let mut output = "".to_owned();
            stdin.read_to_string(&mut output)?;
            let outcome = simple::accepts(&case, &output);
            if outcome.failure() {
                outcome.print_details(config.judge_display_limit(), &mut stderr)?;
                stderr.flush()?;
                Err(JudgeErrorKind::TestFailed(1, 1).into())
            } else {
                Ok(())
            }
        }
        TestCases::Interactive(_) => Err(JudgeErrorKind::ExpectedSimple.into()),
    }
}

pub(crate) fn only_transpile(
    stdout: impl TermOut,
    stderr: impl TermOut,
    config: &Config,
    problem: &str,
    language: Option<&str>,
    force: bool,
) -> JudgeResult<bool> {
    match config.solver_transpilation(language)? {
        None => Ok(false),
        Some(transpilation) => {
            let transpilation = transpilation.expand(problem)?;
            transpilation.run(stdout, stderr, force)?;
            Ok(true)
        }
    }
}

/// Executes the tests.
///
/// # Errors
///
/// Returns `Err` if compilation or execution command fails, or any test fails.
pub(crate) fn judge(params: JudgeParams<impl TermOut, impl TermOut>) -> JudgeResult<()> {
    fn judge_all<
        C: TestCase,
        O: Outcome + Send + 'static,
        F: Future<Item = O, Error = io::Error> + Send + 'static,
    >(
        mut stdout: impl TermOut,
        mut stderr: impl TermOut,
        jobs: NonZeroUsize,
        display_limit: Option<usize>,
        cases: Vec<C>,
        solver: &Arc<JudgingCommand>,
        judge: fn(&C, &Arc<JudgingCommand>) -> JudgeResult<F>,
    ) -> JudgeResult<()> {
        let num_cases = cases.len();
        let names = cases.iter().map(|c| c.name()).collect::<Vec<_>>();
        let name_max_width = names.iter().map(|s| stdout.str_width(s)).max().unwrap_or(0);

        let mut cases = names
            .into_iter()
            .zip_eq(cases)
            .enumerate()
            .map(|(i, (name, case))| (i, name, case));

        let (tx, rx) = futures::sync::mpsc::channel(num_cases);
        let mut runtime = Runtime::new()?;
        {
            let tx = tx.clone();
            runtime.spawn(ctrl_c().then(move |r| {
                let (dummy_i, dummy_name) = (num_cases, Arc::new("".to_owned()));
                let _ = tx.send((dummy_i, dummy_name, r)).wait();
                Ok(())
            }));
        }
        for _ in 0..jobs.get() {
            spawn_head(&mut cases, &mut runtime, tx.clone(), solver, judge)?;
        }
        write!(stderr, "0/{} test finished (0 failure)", num_cases)?;
        if !stderr.supports_color() {
            writeln!(stderr)?;
        }
        stderr.flush()?;
        let (mut num_finished, mut num_failures) = (0, 0);
        let mut outcomes = rx
            .take(num_cases as u64)
            .then::<_, JudgeResult<_>>(|r| {
                let (i, name, r) = r.unwrap();
                let outcome = r?;
                num_finished += 1;
                if outcome.failure() {
                    num_failures += 1;
                }
                if stderr.supports_color() {
                    stderr.write_str("\x1b[0G\x1b[2K")?;
                }
                let color = match num_failures {
                    0 => 10,
                    _ => 9,
                };
                stderr.with_reset(|o| {
                    write!(
                        o.fg(color)?,
                        "{}/{} {} finished ({})",
                        num_finished,
                        num_cases,
                        if num_finished > 1 { "tests" } else { "test" },
                        plural!(num_failures, "failure", "failures"),
                    )
                })?;
                if !stderr.supports_color() {
                    writeln!(stderr)?;
                }
                stderr.flush()?;
                spawn_head(&mut cases, &mut runtime, tx.clone(), solver, judge)?;
                Ok((i, name, outcome))
            })
            .collect()
            .wait()?;
        if stderr.supports_color() {
            writeln!(stderr)?;
            stderr.flush()?;
        }
        outcomes.sort_by_key(|(i, _, _)| *i);
        let _ = runtime.shutdown_now().wait();

        if num_failures == 0 {
            for (i, name, outcome) in outcomes {
                outcome.print_title(&mut stdout, i + 1, num_cases, &name, Some(name_max_width))?;
            }
            writeln!(
                stdout,
                "All of the {} passed.",
                plural!(num_cases, "test", "tests")
            )?;
            stdout.flush()?;
            Ok(())
        } else {
            for (i, name, outcome) in outcomes {
                writeln!(stdout)?;
                outcome.print_title(&mut stdout, i + 1, num_cases, &name, None)?;
                outcome.print_details(display_limit, &mut stdout)?;
            }
            stdout.flush()?;
            Err(JudgeErrorKind::TestFailed(num_failures, num_cases).into())
        }
    }

    fn spawn_head<
        C: TestCase,
        O: Outcome + Send + 'static,
        F: Future<Item = O, Error = io::Error> + Send + 'static,
    >(
        mut cases: impl Iterator<Item = (usize, Arc<String>, C)>,
        runtime: &mut Runtime,
        tx: futures::sync::mpsc::Sender<(usize, Arc<String>, io::Result<O>)>,
        solver: &Arc<JudgingCommand>,
        judge: fn(&C, &Arc<JudgingCommand>) -> JudgeResult<F>,
    ) -> JudgeResult<()> {
        if let Some((i, name, case)) = cases.next() {
            runtime.spawn(judge(&case, solver)?.then(move |r| {
                let _ = tx.send((i, name, r)).wait(); // `rx` may be dropped
                Ok(())
            }));
        }
        Ok(())
    }

    fn ctrl_c<T>() -> impl Future<Item = T, Error = io::Error> {
        tokio_signal::ctrl_c()
            .flatten_stream()
            .take(1)
            .into_future()
            .map_err(|(e, _)| e)
            .and_then::<_, io::Result<T>>(|_| {
                Err(io::Error::new(io::ErrorKind::Interrupted, "Interrupted"))
            })
    }

    let JudgeParams {
        mut stdout,
        mut stderr,
        config,
        problem,
        language,
        force_compile,
        jobs,
    } = params;

    let (cases, paths_formatted) = config.testcase_loader().load_merging(problem)?;
    let jobs = jobs
        .or_else(|| config.judge_jobs())
        .unwrap_or_else(|| NonZeroUsize::new(1).unwrap());
    let display_limit = config.judge_display_limit();
    let tester_transpilations = cases.interactive_tester_transpilations();
    let tester_compilations = cases.interactive_tester_compilations();
    let solver = config.solver(language)?.expand(&problem)?;
    let solver_transpilation = match config.solver_transpilation(language)? {
        Some(transpilation) => Some(transpilation.expand(&problem)?),
        None => None,
    };
    let solver_compilation = match config.solver_compilation(language)? {
        Some(compilation) => Some(compilation.expand(&problem)?),
        None => None,
    };

    for tester_transpilation in tester_transpilations {
        tester_transpilation.run(&mut stdout, &mut stderr, force_compile)?;
        writeln!(stdout)?;
    }
    for tester_compilation in tester_compilations {
        tester_compilation.run(&mut stdout, &mut stderr, force_compile)?;
        writeln!(stdout)?;
    }
    if let Some(solver_transpilation) = solver_transpilation {
        solver_transpilation.run(&mut stdout, &mut stderr, force_compile)?;
        writeln!(stdout)?;
    }
    if let Some(solver_compilation) = solver_compilation {
        solver_compilation.run(&mut stdout, &mut stderr, force_compile)?;
        writeln!(stdout)?;
    }

    solver.write_info(&mut stdout, &paths_formatted)?;
    stdout.flush()?;

    let solver = Arc::new(solver);
    match cases {
        TestCases::Simple(cases) => judge_all(
            stdout,
            stderr,
            jobs,
            display_limit,
            cases,
            &solver,
            simple::judge,
        ),
        TestCases::Interactive(cases) => judge_all(
            stdout,
            stderr,
            jobs,
            display_limit,
            cases,
            &solver,
            interactive::judge,
        ),
    }
}

pub(crate) struct JudgeParams<'a, O: TermOut, E: TermOut> {
    pub stdout: O,
    pub stderr: E,
    pub config: &'a Config,
    pub problem: &'a str,
    pub language: Option<&'a str>,
    pub force_compile: bool,
    pub jobs: Option<NonZeroUsize>,
}

pub(self) trait Outcome: fmt::Display {
    fn failure(&self) -> bool;
    fn color(&self) -> u8;
    fn print_details(&self, display_limit: Option<usize>, out: impl TermOut) -> io::Result<()>;

    fn print_title(
        &self,
        mut out: impl TermOut,
        i: impl DisplayableNum,
        n: impl DisplayableNum,
        name: &str,
        name_width: Option<usize>,
    ) -> io::Result<()> {
        if name_width.is_some() {
            out.write_spaces(n.num_digits() - i.num_digits())?;
        }
        out.with_reset(|o| write!(o.bold()?, "{}/{} ({})", i, n, name))?;
        let l = out.str_width(name);
        let name_width = name_width.unwrap_or(0);
        out.write_spaces(cmp::max(name_width, l) - l + 1)?;
        out.with_reset(|o| writeln!(o.fg(self.color())?, "{}", self))
    }
}

trait DisplayableNum: fmt::Display + Copy {
    fn num_digits(self) -> usize;
}

impl DisplayableNum for usize {
    fn num_digits(mut self) -> usize {
        let mut r = 1;
        while self > 9 {
            self /= 10;
            r += 1;
        }
        r
    }
}

pub(self) fn writeln_size(mut out: impl WriteAnsi, size: usize) -> io::Result<()> {
    let gib = size / 2usize.pow(30);
    let mib = (size / 2usize.pow(20)) & 0x3ff;
    let kib = (size / 2usize.pow(10)) & 0x3ff;
    let b = size & 0x3ff;
    out.with_reset(|out| {
        out.fg(11)?.bold()?;
        match (gib, mib, kib, b) {
            (0, 0, 0, b) => writeln!(out, "{}B", b),
            (0, 0, k, b) => writeln!(out, "{}.{}KiB", k, b / 0x67),
            (0, m, k, _) => writeln!(out, "{}.{}MiB", m, k / 0x67),
            (g, m, _, _) => writeln!(out, "{}.{}GiB", g, m / 0x67),
        }
    })
}
