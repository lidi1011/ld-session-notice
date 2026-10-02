const SECRET_KEYS: [&str; 5] = ["token", "apikey", "secret", "password", "authorization"];

pub fn redact(input: &str) -> String {
    let mut output = input.to_string();
    for key in SECRET_KEYS {
        output = redact_key_value(&output, key);
    }
    output
}

fn redact_key_value(input: &str, key: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(index) = lower.find(key) {
            let after_key = &line[index + key.len()..];
            if after_key.trim_start().starts_with(['=', ':']) {
                out.push_str(&line[..index + key.len()]);
                out.push_str("=<redacted>");
            } else {
                out.push_str(line);
            }
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !input.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redacts_known_secret_keys() {
        let text = "token=abc\napikey: xyz\npassword = letmein\nauthorization: bearer value";
        let redacted = redact(text);
        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("xyz"));
        assert!(!redacted.contains("letmein"));
        assert!(!redacted.contains("bearer value"));
        assert!(redacted.contains("<redacted>"));
    }
}
