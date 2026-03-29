use reflect_core::types::{EvalError, EvalSignal, Severity};
use regex::Regex;

pub fn parse_tsc_output(output: &str) -> EvalSignal {
    let errors = extract_errors(output);
    let passed = errors.is_empty();
    let summary = if passed {
        "no errors".into()
    } else {
        format!("{} errors", errors.len())
    };

    EvalSignal {
        evaluator: "tsc".into(),
        passed,
        summary,
        errors,
    }
}

fn extract_errors(output: &str) -> Vec<EvalError> {
    let re = Regex::new(r"^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$").unwrap();

    output
        .lines()
        .filter_map(|line| {
            re.captures(line).map(|cap| EvalError {
                file: Some(cap[1].to_string()),
                line: cap[2].parse().ok(),
                column: cap[3].parse().ok(),
                code: Some(cap[4].to_string()),
                message: cap[5].to_string(),
                severity: Severity::Error,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/tsc_output.txt");

    #[test]
    fn parses_type_errors() {
        let signal = parse_tsc_output(FIXTURE);
        assert!(!signal.passed);
        assert_eq!(signal.evaluator, "tsc");
        assert_eq!(signal.summary, "3 errors");
        assert_eq!(signal.errors.len(), 3);

        // First error
        assert_eq!(
            signal.errors[0].file,
            Some("src/components/Auth.tsx".into())
        );
        assert_eq!(signal.errors[0].line, Some(12));
        assert_eq!(signal.errors[0].column, Some(5));
        assert_eq!(signal.errors[0].code, Some("TS2322".into()));
        assert_eq!(
            signal.errors[0].message,
            "Type 'string' is not assignable to type 'number'."
        );
        assert_eq!(signal.errors[0].severity, Severity::Error);

        // Second error
        assert_eq!(
            signal.errors[1].file,
            Some("src/components/Auth.tsx".into())
        );
        assert_eq!(signal.errors[1].line, Some(24));
        assert_eq!(signal.errors[1].column, Some(10));
        assert_eq!(signal.errors[1].code, Some("TS7006".into()));
        assert_eq!(
            signal.errors[1].message,
            "Parameter 'e' implicitly has an 'any' type."
        );

        // Third error
        assert_eq!(signal.errors[2].file, Some("src/utils/format.ts".into()));
        assert_eq!(signal.errors[2].line, Some(8));
        assert_eq!(signal.errors[2].column, Some(1));
        assert_eq!(signal.errors[2].code, Some("TS2304".into()));
        assert_eq!(signal.errors[2].message, "Cannot find name 'moment'.");
    }

    #[test]
    fn parses_clean_output() {
        let signal = parse_tsc_output("");
        assert!(signal.passed);
        assert_eq!(signal.evaluator, "tsc");
        assert_eq!(signal.summary, "no errors");
        assert!(signal.errors.is_empty());
    }
}
