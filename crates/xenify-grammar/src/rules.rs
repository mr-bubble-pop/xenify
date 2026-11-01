//! Xenify's own grammar rules, added alongside Harper's.
//!
//! Rules written in Harper's Weir language live in `rules/*.weir`; each
//! carries `test "wrong" "right"` and `allows "…"` lines that `cargo test`
//! runs. The rules are our own work: error patterns come from general
//! grammar knowledge and published lists of common mistakes (named in each
//! file's header comment); no rule text is copied from other checkers.

use harper_core::linting::LintGroup;
use harper_core::weir::WeirLinter;

macro_rules! weir_rules {
    ($($name:literal),* $(,)?) => {
        /// (rule name, Weir source) for every rule in `rules/`.
        pub const WEIR_RULES: &[(&str, &str)] = &[
            $(($name, include_str!(concat!("../rules/", $name, ".weir")))),*
        ];
    };
}

weir_rules![
    "PubicPublic",
    "MangerManager",
    "ProstrateProstate",
    "DefiantlyDefinitely",
    "AssesAssess",
    "ShitShift",
    "UntiedUnited",
    "UntiedForUnited",
    "BarleyBarely",
    "ExistingExiting",
    "FormFrom",
    "FromForm",
    "SingSign",
    "LostLot",
    "ComplimentComplement",
    "ComplementForCompliment",
    "PrincipalPrinciple",
    "PrinciplesForPrincipals",
    "PrinciplesPlural",
    "StationeryForStationary",
    "ComplementsForCompliments",
    "AcceptExcept",
    "ExceptAccept",
    "ComparisonThen",
    "LayingLying",
    "NipInTheBud",
    "FollowUpHyphen",
    "ThanksYou",
    "InCase",
    "EncaseInCase",
    "ManyPeople",
    "LessForFewer",
    "WhatWasLeft",
    "WhatIsLeft",
    "YourForYoure",
    "WhosForWhose",
    "WeatherForWhether",
    "LooseForLose",
    "OhDear",
];

/// Adds and enables our rules in `group`.
pub fn add_to(group: &mut LintGroup) {
    for (name, src) in WEIR_RULES {
        let linter = match WeirLinter::new(src) {
            Ok(l) => l,
            Err(e) => {
                // A broken rule must not break checking; tests catch it.
                eprintln!("xenify-wp-check: rule {name} doesn't parse: {e:?}");
                continue;
            }
        };
        let added = match linter.into_sentence_linter() {
            Ok(l) => group.add_sentence_expr_linter(name, l),
            Err(l) => match l.into_chunk_linter() {
                Ok(l) => group.add_chunk_expr_linter(name, l),
                Err(l) => group.add(name, l),
            },
        };
        if added {
            group.config.set_rule_enabled(name, true);
        }
    }
    for name in crate::grammar::add_to(group) {
        group.config.set_rule_enabled(name, true);
    }
    if group.add("SoundAlike", crate::sound_alike::SoundAlike) {
        group.config.set_rule_enabled("SoundAlike", true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_file_is_listed() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/rules");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "weir"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        files.sort();
        let mut listed: Vec<String> = WEIR_RULES.iter().map(|(n, _)| n.to_string()).collect();
        listed.sort();
        assert_eq!(files, listed);
    }

    #[test]
    fn weir_rules_pass_their_tests() {
        let mut failures = Vec::new();
        for (name, src) in WEIR_RULES {
            let mut l = WeirLinter::new(src).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(l.count_tests() > 0, "{name} has no tests");
            for r in l.run_tests() {
                failures.push(format!("{name}: expected {:?}, got {:?}", r.expected, r.got));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn rule_names_do_not_clash_with_harpers() {
        let curated = LintGroup::new_curated(
            std::sync::Arc::new(harper_core::spell::FstDictionary::curated()),
            harper_core::Dialect::American,
        );
        for name in WEIR_RULES.iter().map(|(n, _)| *n).chain(crate::grammar::NAMES.iter().copied()) {
            assert!(!curated.contains_key(name), "{name} is already a Harper rule");
        }
    }
}
