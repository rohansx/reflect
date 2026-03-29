use reflect_core::types::{EvalError, EvalSignal, Severity};
use regex::Regex;

pub fn parse_eslint_output(output: &str) -> EvalSignal {
    let (error_count, warning_count) = extract_counts(output);
    let passed = error_count == 0 && warning_count == 0;
    let summary = format!("{} errors, {} warnings", error_count, warning_count);
    let errors = extract_errors(output);

    EvalSignal {
        evaluator: "eslint".into(),
        passed,
        summary,
        errors,
    }
}

fn extract_counts(output: &str) -> (u32, u32) {
    let re = Regex::new(r"(\d+) errors?,\s*(\d+) warnings?\)").unwrap();
    output
        .lines()
        .find_map(|line| {
            re.captures(line).map(|cap| {
                (
                    cap[1].parse::<u32>().unwrap_or(0),
                    cap[2].parse::<u32>().unwrap_or(0),
                )
            })
        })
        .unwrap_or((0, 0))
}

fn extract_errors(output: &str) -> Vec<EvalError> {
    let file_re = Regex::new(r"^(\S.+)$").unwrap();
    let diag_re =
        Regex::new(r"^\s+(\d+):(\d+)\s+(error|warning)\s+(.+?)\s{2,}(\S+)\s*$").unwrap();
    let summary_re = Regex::new(r"^\u{2716}").unwrap();

    let mut errors = Vec::new();
    let mut current_file: Option<String> = None;

    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if summary_re.is_match(line) {
            break;
        }
        if let Some(cap) = diag_re.captures(line) {
            let severity = match &cap[3] {
                "error" => Severity::Error,
                _ => Severity::Warning,
            };
            errors.push(EvalError {
                file: current_file.clone(),
                line: cap[1].parse().ok(),
                column: cap[2].parse().ok(),
                code: Some(cap[5].to_string()),
                message: cap[4].to_string(),
                severity,
            });
        } else if file_re.is_match(line) && !line.starts_with(' ') {
            current_file = Some(line.trim().to_string());
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/eslint_output.txt");

    #[test]
    fn parses_errors_and_warnings() {
        let signal = parse_eslint_output(FIXTURE);
        assert!(!signal.passed);
        assert_eq!(signal.evaluator, "eslint");
        assert_eq!(signal.errors.len(), 5);

        // First file, first error
        assert_eq!(
            signal.errors[0].file,
            Some("/home/user/project/src/components/Auth.tsx".into())
        );
        assert_eq!(signal.errors[0].line, Some(12));
        assert_eq!(signal.errors[0].column, Some(5));
        assert_eq!(signal.errors[0].code, Some("no-unused-vars".into()));
        assert_eq!(signal.errors[0].severity, Severity::Error);

        // First file, warning
        assert_eq!(
            signal.errors[2].file,
            Some("/home/user/project/src/components/Auth.tsx".into())
        );
        assert_eq!(signal.errors[2].line, Some(45));
        assert_eq!(signal.errors[2].column, Some(3));
        assert_eq!(signal.errors[2].code, Some("no-console".into()));
        assert_eq!(signal.errors[2].severity, Severity::Warning);

        // Second file
        assert_eq!(
            signal.errors[3].file,
            Some("/home/user/project/src/utils/format.ts".into())
        );
        assert_eq!(signal.errors[3].line, Some(8));
        assert_eq!(signal.errors[3].column, Some(1));
        assert_eq!(signal.errors[3].code, Some("no-undef".into()));
        assert_eq!(signal.errors[3].severity, Severity::Error);

        // Second file, warning
        assert_eq!(signal.errors[4].severity, Severity::Warning);
        assert_eq!(signal.errors[4].code, Some("no-unused-vars".into()));
    }

    #[test]
    fn parses_clean_output() {
        let output = "";
        let signal = parse_eslint_output(output);
        assert!(signal.passed);
        assert_eq!(signal.evaluator, "eslint");
        assert!(signal.errors.is_empty());
    }
}
