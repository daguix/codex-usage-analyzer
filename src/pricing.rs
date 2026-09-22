use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Rates {
    pub input: f64,
    pub cached: f64,
    pub output: f64,
}

#[derive(Debug)]
pub struct Pricing {
    rates: HashMap<&'static str, Rates>,
}

impl Default for Pricing {
    fn default() -> Self {
        let mut rates = HashMap::new();
        for (name, input, cached, output) in [
            ("gpt-6-astra", 10.0, 1.0, 50.0),
            ("gpt-5.6-sol", 4.0, 0.4, 20.0),
            ("gpt-5.6-terra", 2.0, 0.2, 12.0),
            ("gpt-5.6-luna", 0.2, 0.02, 1.2),
            ("gpt-5.2", 1.75, 0.175, 14.0),
            ("gpt-5.4", 2.5, 0.25, 15.0),
            ("gpt-5.4-mini", 0.75, 0.075, 4.5),
            ("gpt-5.5", 5.0, 0.5, 30.0),
            ("gpt-5.2-pro", 21.0, 21.0, 168.0),
            ("gpt-5.4-pro", 30.0, 30.0, 180.0),
            ("gpt-5.5-pro", 30.0, 30.0, 180.0),
            ("gpt-5", 5.0, 0.5, 30.0),
            ("gpt-5.1-codex-max", 1.25, 0.125, 10.0),
            ("gpt-5.1-codex", 1.25, 0.125, 10.0),
            ("gpt-5.1-codex-mini", 0.25, 0.025, 2.0),
            ("gpt-5.2-codex", 1.75, 0.175, 14.0),
            ("gpt-5-codex", 1.75, 0.175, 14.0),
            ("gpt-5.3-codex", 1.75, 0.175, 14.0),
        ] {
            rates.insert(
                name,
                Rates {
                    input,
                    cached,
                    output,
                },
            );
        }
        Self { rates }
    }
}

impl Pricing {
    pub fn rates_for(&self, model: &str) -> Option<Rates> {
        let cleaned = model.trim();
        if let Some(rates) = self.rates.get(cleaned) {
            return Some(*rates);
        }
        if cleaned.ends_with(')')
            && let Some((base, _)) = cleaned.split_once(" (")
            && let Some(rates) = self.rates.get(base)
        {
            return Some(*rates);
        }
        let undated = strip_dated_suffix(cleaned);
        if let Some(rates) = self.rates.get(undated) {
            return Some(*rates);
        }
        let alias = match cleaned {
            "codex-auto-review" => "gpt-5.6-luna",
            "gpt-5.6" => "gpt-5.6-sol",
            "gpt-5-codex" => "gpt-5.2-codex",
            "gpt-5" => "gpt-5.5",
            "gpt-5-pro" => "gpt-5.5-pro",
            _ => return None,
        };
        self.rates.get(alias).copied()
    }
}

fn strip_dated_suffix(value: &str) -> &str {
    if value.len() < 11 {
        return value;
    }
    let split = value.len() - 11;
    let suffix = &value[split..];
    let bytes = suffix.as_bytes();
    if bytes[0] == b'-'
        && bytes[1..5].iter().all(u8::is_ascii_digit)
        && bytes[5] == b'-'
        && bytes[6..8].iter().all(u8::is_ascii_digit)
        && bytes[8] == b'-'
        && bytes[9..11].iter().all(u8::is_ascii_digit)
    {
        &value[..split]
    } else {
        value
    }
}
