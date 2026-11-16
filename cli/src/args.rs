//! The arguments after a command, by the same rules for every command:
//! positional arguments, switches (--wit) and options with a value
//! (--reason <text>, or --reason=<text>), in any order, each at most once.

/// What a command takes.
pub struct Spec {
    pub usage: &'static str,
    /// Positional arguments: at least `required`, at most `positional`.
    pub required: usize,
    pub positional: usize,
    pub switches: &'static [&'static str],
    pub options: &'static [&'static str],
}

impl Spec {
    /// A command that takes nothing; the others start from it.
    pub const fn bare(usage: &'static str) -> Spec {
        Spec { usage, required: 0, positional: 0, switches: &[], options: &[] }
    }

    pub fn parse<'a>(&self, rest: &'a [String]) -> anyhow::Result<Args<'a>> {
        let usage = self.usage;
        let mut args = Args::default();
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            if !a.starts_with('-') {
                if args.positional.len() == self.positional {
                    anyhow::bail!("usage: {usage}");
                }
                args.positional.push(a);
                continue;
            }
            let (name, inline) = match a.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (a.as_str(), None),
            };
            if let Some(&switch) = self.switches.iter().find(|s| **s == name) {
                if inline.is_some() || args.given(switch) {
                    anyhow::bail!("usage: {usage}");
                }
                args.switches.push(switch);
            } else if let Some(&option) = self.options.iter().find(|o| **o == name) {
                if args.given(option) {
                    anyhow::bail!("{option} is given twice; usage: {usage}");
                }
                let value = match inline {
                    Some(v) => v,
                    None => it.next().ok_or_else(|| anyhow::anyhow!("{option} needs a value; usage: {usage}"))?,
                };
                args.options.push((option, value));
            } else {
                anyhow::bail!("unknown option {a:?}; usage: {usage}");
            }
        }
        if args.positional.len() < self.required {
            anyhow::bail!("usage: {usage}");
        }
        Ok(args)
    }
}

/// The arguments a `Spec` read.
#[derive(Default)]
pub struct Args<'a> {
    positional: Vec<&'a str>,
    switches: Vec<&'static str>,
    options: Vec<(&'static str, &'a str)>,
}

impl<'a> Args<'a> {
    pub fn first(&self) -> Option<&'a str> {
        self.positional.first().copied()
    }

    pub fn switch(&self, name: &str) -> bool {
        self.switches.contains(&name)
    }

    pub fn option(&self, name: &str) -> Option<&'a str> {
        self.options.iter().find(|(o, _)| *o == name).map(|(_, v)| *v)
    }

    fn given(&self, name: &str) -> bool {
        self.switch(name) || self.option(name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec { positional: 1, switches: &["--wit"], options: &["--reason"], ..Spec::bare("appos x [a] [--wit] [--reason <text>]") };

    fn parse(a: &[&str]) -> anyhow::Result<(Option<String>, bool, Option<String>)> {
        let rest: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        let args = SPEC.parse(&rest)?;
        Ok((args.first().map(String::from), args.switch("--wit"), args.option("--reason").map(String::from)))
    }

    #[test]
    fn arguments_come_in_any_order() {
        assert_eq!(parse(&[]).unwrap(), (None, false, None));
        assert_eq!(parse(&["--wit", "a", "--reason", "-x"]).unwrap(), (Some("a".into()), true, Some("-x".into())));
        assert_eq!(parse(&["--reason=a=b", "a"]).unwrap(), (Some("a".into()), false, Some("a=b".into())));
        assert_eq!(parse(&["--reason", ""]).unwrap(), (None, false, Some(String::new())));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [&["a", "b"][..], &["--wit", "--wit"], &["--wit=yes"], &["--reason"], &["--reason", "x", "--reason=y"], &["-w"], &["--reasonable"]] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let required = Spec { required: 1, positional: 1, ..Spec::bare("appos use <name>") };
        assert!(required.parse(&[]).is_err());
        assert!(Spec::bare("appos build").parse(&["x".to_string()]).is_err());
    }
}
