use {bincode, cookie, futures, httpsession, regex, serde_json, serde_urlencoded, serde_yaml, toml};
use chrono::{self, DateTime, Local};
use httpsession::UrlError;
use zip::result::ZipError;

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::string::FromUtf8Error;
use std::sync::mpsc::RecvError;

error_chain!{
    links {
        Service(ServiceError, ServiceErrorKind);
        Judge(JudgeError, JudgeErrorKind);
        SuiteFile(SuiteFileError, SuiteFileErrorKind);
        Config(ConfigError, ConfigErrorKind);
    }
}

error_chain! {
    types {
        ServiceError, ServiceErrorKind, ServiceResultExt, ServiceResult;
    }

    links {
        FileIo(FileIoError, FileIoErrorKind);
        CodeReplace(CodeReplaceError, CodeReplaceErrorKind);
        SuiteFile(SuiteFileError, SuiteFileErrorKind);
    }

    foreign_links {
        Bincode(bincode::Error);
        ChronoParse(chrono::ParseError);
        CookieParse(cookie::ParseError);
        HttpSession(httpsession::Error);
        Io(io::Error);
        Recv(RecvError);
        SerdeJson(serde_json::Error);
        SerdeUrlencodedSer(serde_urlencoded::ser::Error);
        Url(UrlError);
        Zip(ZipError);
    }

    errors {
        AlreadyAccepted {
            description("Found an accepted submission")
            display("Found an accepted submission. Add \"--force\" to submit")
        }

        ContestNotBegun(contest_name: String, begins_at: DateTime<Local>) {
            description("Contest has not begun yet")
            display("{} will begin at {}", contest_name, begins_at)
        }

        ContestNotFound(contest_name: String) {
            description("Contest not found")
            display("{} not found", contest_name)
        }

        NoSuchProblem(name: String) {
            description("No such problem")
            display("No such problem: {:?}", name)
        }

        Scrape {
            description("Scraping failed")
            display("Scraping failed")
        }

        Thread {
            description("Thread error")
            display("Thread error")
        }

        Webbrowser(status: ExitStatus) {
            description("Failed to open a URL in the default browser")
            display("{}",
                    if let Some(code) = status.code() {
                        format!("The default browser terminated abnormally with code {}", code)
                    } else {
                        "The default browser terminated abnormally without code (possibly killed)"
                            .to_owned()
                    })
        }
    }
}

error_chain! {
    types {
        JudgeError, JudgeErrorKind, JudgeResultExt, JudgeResult;
    }

    links {
        SuiteFile(SuiteFileError, SuiteFileErrorKind);
    }

    foreign_links {
        Io(io::Error);
        Recv(RecvError);
        FuturesCanceled(futures::Canceled);
    }

    errors {
        CommandNotFound(command: String) {
            description("Command not found")
            display("No such command: {:?}", command)
        }

        Compile(status: ExitStatus) {
            description("Compilation failed")
            display("The compilation command terminated abnormally {}",
                    if let Some(code) = status.code() { format!("with code {}", code) }
                    else { "without code".to_owned() })
        }

        TestFailure(n: usize, d: usize) {
            description("Test failed")
            display("{}/{} Test{} failed", n, d, if *n > 0 { "s" } else { "" })
        }
    }
}

error_chain! {
    types {
        SuiteFileError, SuiteFileErrorKind, SuiteFileResultExt, SuiteFileResult;
    }

    links {
        FileIo(FileIoError, FileIoErrorKind);
    }

    foreign_links {
        SerdeJson(serde_json::Error);
        SerdeYaml(serde_yaml::Error);
        TomlDe(toml::de::Error);
        TomlSer(toml::ser::Error);
    }

    errors {
        DifferentTypesOfSuites {
            description("Different types of suites")
            display("Different types of suites")
        }

        Unsubmittable{
            description("This problem is unsubmittable")
            display("This problem is unsubmittable")
        }

        SuiteIsNotSimple {
            description("Target suite is not \"simple\" type")
            display("Target suite is not \"simple\" type")
        }
    }
}

error_chain! {
    types {
        ConfigError, ConfigErrorKind, ConfigResultExt, ConfigResult;
    }

    links {
        FileIo(FileIoError, FileIoErrorKind);
        CodeReplace(CodeReplaceError, CodeReplaceErrorKind);
    }

    foreign_links {
        Io(io::Error);
        Regex(regex::Error);
        SerdeYaml(serde_yaml::Error);
        Template(TemplateError);
    }

    errors {
        ConfigFileNotFound {
            description("\"snowchains.yaml\" not found")
            display("\"snowchains.yaml\" not found")
        }

        LanguageNotSpecified {
            description("Language not specified")
            display("Language not specified")
        }

        NoSuchLanguage(name: String) {
            description("Language not found")
            display("No such language: \"{}\"", name)
        }

        PropertyNotSet(property: &'static str) {
            description("Property not set")
            display("Property not set: \"{}\"", property)
        }
    }
}

error_chain! {
    types {
        CodeReplaceError, CodeReplaceErrorKind, CodeReplaceResultExt, CodeReplaceResult;
    }

    foreign_links {
        Regex(regex::Error);
        Template(TemplateError);
        FromUtf8(FromUtf8Error);
    }

    errors {
        RegexGroupOutOfBounds(i: usize) {
            description("Regex group out of bounds")
            display("Regex group out of bounds: {}", i)
        }

        NoMatch(regex: String) {
            description("No match")
            display("No match: {:?}", regex)
        }
    }
}

error_chain! {
    types {
        TemplateError, TemplateErrorKind, TemplateResultExt, TemplateResult;
    }

    links {
        FileIo(FileIoError, FileIoErrorKind);
    }

    errors {
        InvalidVariable(var: String) {
            description("Invalid variable")
            display("Invalid variable: {:?}", var)
        }

        Syntax(whole: String) {
            description("Syntax error")
            display("Syntax error: {:?}", whole)
        }

        NoSuchSpecifier(whole: String, specifier: String, expected: &'static [&'static str]) {
            description("No such format specifier")
            display("No such format specifier {:?} (expected {:?}): {:?}",
                    specifier, expected, whole)
        }

        NoSuchVariable(whole: String, keyword: String, expected: Vec<String>) {
            description("Variable not found")
            display("No such variable {:?} (expected {:?} + environment variables): {:?}",
                    keyword, expected, whole)
        }

        NonUtf8EnvVar(var: String) {
            description("Non UTF-8 environment variable")
            display("Non UTF-8 environment variable: {:?}", var)
        }
    }
}

error_chain! {
    types {
        FileIoError, FileIoErrorKind, FileIoResultExt, FileIoResult;
    }

    foreign_links {
        Io(io::Error);
    }

    errors {
        OpenInReadOnly(path: PathBuf) {
            description("Failed to open a file in read-only mode")
            display("An IO error occurred while opening {} in read-only mode", path.display())
        }

        OpenInWriteOnly(path: PathBuf) {
            description("Failed to open a file in write-only mode")
            display("An IO error occurred while opening {} in write-only mode", path.display())
        }

        Write(path: PathBuf) {
            description("Failed to write data to a file")
            display("Failed to write to {}", path.display())
        }

        Expand(path: String) {
            description("Failed to expand a path")
            display("Failed to expand {:?}", path)
        }

        HomeDirNotFound {
            description("Home directory not found")
            display("Home directory not found")
        }

        UnsupportedUseOfTilde {
            description("Unsupported use of \"~\"")
            display("Unsupported use of \"~\"")
        }
    }
}
