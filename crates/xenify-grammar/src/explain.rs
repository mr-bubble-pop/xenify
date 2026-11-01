//! "Why?" explanations: a plain-English account of a rule with wrong →
//! right examples, so the checker teaches as well as corrects.

use crate::Feature;

/// What a rule is about, for the "Why?" panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Explanation {
    pub title: String,
    pub text: String,
    /// (wrong, right) pairs.
    pub examples: Vec<(String, String)>,
}

/// Plain-English explanations of Xenify's Rust rules and style checks, with examples.
/// (rule, title, explanation, examples)
type Entry = (&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)]);

const OWN: &[Entry] = &[
    (
        "PronounCaseInPairs",
        "I or me? He or him?",
        "When a pronoun is paired with someone else, use the form you'd use if it stood alone. Try removing the other person: \"Her is taking the dogs\" sounds wrong, so it's \"She and Marcus are\". \"Mom made dinner for I\" sounds wrong, so it's \"for my brother and me\". After words like between, for, with, to and told, use me/him/her/us/them.",
        &[
            ("Her and Marcus are coming.", "She and Marcus are coming."),
            ("This is between you and I.", "This is between you and me."),
            ("Me and Sam finished early.", "Sam and I finished early."),
        ],
    ),
    (
        "ParticipleAsPast",
        "Seen or saw? Done or did?",
        "Words like seen, done, gone and taken need a helper verb (have, has, had) in standard written English. On their own, use the simple past: saw, did, went, took. \"I seen it\" is common in speech and in many dialects; your writing profile decides whether it's pointed out.",
        &[("I seen him there.", "I saw him there."), ("They done it again.", "They did it again.")],
    ),
    (
        "ItsForItIs",
        "Its or it's?",
        "It's = it is or it has (the apostrophe marks the missing letters). Its = belonging to it, like his or her, with no apostrophe. Test: if you can say \"it is\" or \"it has\" instead, write it's.",
        &[("Its raining today.", "It's raining today."), ("Its been a long week.", "It's been a long week.")],
    ),
    (
        "ItIsForIts",
        "It's or its?",
        "Its (no apostrophe) means belonging to it: the dog wagged its tail. It's always means it is or it has. Test: if \"it is\" doesn't fit, write its.",
        &[
            ("The park closes and it's gates are locked.", "The park closes and its gates are locked."),
            ("Each module has it's own endpoint.", "Each module has its own endpoint."),
        ],
    ),
    (
        "ThereIsPlural",
        "There is or there are?",
        "After \"there\", the verb matches the thing that follows: there is one reason, there are many reasons.",
        &[
            ("There is many reasons to stay.", "There are many reasons to stay."),
            ("There was several problems.", "There were several problems."),
        ],
    ),
    (
        "DurationPreposition",
        "In, for or since with time",
        "For a time from now, use in (in 10 minutes). For a length of time, use for (for five years). Since takes a starting point, not a length (since 2019, since Monday).",
        &[
            ("I'll call you on 10 minutes.", "I'll call you in 10 minutes."),
            ("She has lived here since five years.", "She has lived here for five years."),
        ],
    ),
    (
        "LookForwardToIng",
        "Look forward to + -ing",
        "In \"look forward to\", the \"to\" is a preposition, so the verb after it takes -ing, just as you'd say \"look forward to the meeting\".",
        &[("I look forward to hear from you.", "I look forward to hearing from you.")],
    ),
    (
        "QuestionMark",
        "Questions end with ?",
        "A direct question (one that asks the reader something) ends with a question mark. Polite requests like \"Could you send it.\" can end with a full stop.",
        &[("How are you today.", "How are you today?")],
    ),
    (
        "NoVerb",
        "A sentence needs a verb",
        "Every full sentence needs a verb: a word saying what the subject does or is (went, is, has). Without one, a word is usually missing.",
        &[("The cat for walk.", "The cat went for a walk.")],
    ),
    (
        "BoughtBrought",
        "Bought or brought?",
        "Bought is the past of buy (paid for). Brought is the past of bring (carried along). You bring things to a place or a person.",
        &[("I bought snacks to the party.", "I brought snacks to the party.")],
    ),
    (
        "RatherThen",
        "Rather … than",
        "After would rather, the comparison uses than: \"I'd rather walk than drive.\" Then is about time or order.",
        &[("I'd rather be early, then late.", "I'd rather be early than late.")],
    ),
    (
        "LeadForLed",
        "Lead or led?",
        "The past of lead (to guide) is led, which sounds like the metal lead but is spelt with no a: \"Yesterday she led the team.\" Lead is the present tense: \"Today she leads the team.\"",
        &[
            ("Yesterday she lead the team.", "Yesterday she led the team."),
            ("The project was lead by Anna.", "The project was led by Anna."),
        ],
    ),
    (
        "OffForOf",
        "Of or off",
        "`Off` means away or not on (`turn off the light`). After words like `ways`, `lot` or `instead`, the word you want is `of`.",
        &[("New ways off working.", "New ways of working.")],
    ),
    (
        "WereForWeAre",
        "We're or were",
        "`We're` is short for `we are`. After `there` or a subject like `they`, the word you want is `were`, the past of `are`.",
        &[("There we're three cakes on the plate.", "There were three cakes on the plate.")],
    ),
    (
        "TitleNameAgreement",
        "Verb after a name",
        "One named person is the subject, so a present-tense verb takes -s (or use the past tense).",
        &[("Mr Monkey sit at the table.", "Mr Monkey sits at the table.")],
    ),
    (
        "DoubledWord",
        "Word typed twice",
        "The same word appears twice in a row, usually a slip while typing or editing. Delete one.",
        &[("My three dogs dogs bark.", "My three dogs bark.")],
    ),
    (
        "LyHyphen",
        "No hyphen after an -ly word",
        "Words ending in -ly (highly, newly, poorly) already show they describe the next word, so no hyphen joins them: \"a highly skilled team\". Other compounds before a noun do take one: \"a well-known author\".",
        &[("They hired a highly-skilled team.", "They hired a highly skilled team.")],
    ),
    (
        "HyphenAfterNoun",
        "Compounds after the noun",
        "Before a noun, a compound takes hyphens: \"a well-known author\", \"up-to-date figures\". After the noun (\"the author is well known\") most style guides drop them. Some keep them; either is understood.",
        &[("The author is well-known.", "The author is well known.")],
    ),
    (
        "NameCapital",
        "Names take a capital letter",
        "People's names, pets' names and other names start with a capital letter, wherever they are in the sentence.",
        &[("Hi, my name is troy.", "Hi, my name is Troy.")],
    ),
    (
        "PluralAfterMany",
        "Many, two, several… take a plural",
        "Words that count more than one (many, several, few, two, these) go with a plural: many women, two children, several people.",
        &[("Many woman are smart.", "Many women are smart.")],
    ),
    (
        "SingularAfterA",
        "A or one before a plural",
        "A, an, one, each and every go with a single thing: a woman, one child. Women, men, children, feet and teeth are already plural.",
        &[("Never underestimate a women.", "Never underestimate a woman.")],
    ),
    (
        "RunOnSentence",
        "Two sentences with nothing joining them",
        "Each sentence needs its own full stop, or a joining word (and, but, so, because) to link it to the next. Read it aloud: where you'd pause and start again, a full stop goes.",
        &[("I went home I ate dinner.", "I went home. I ate dinner.")],
    ),
    (
        "MissingSmallWord",
        "A small word is missing",
        "Some words need a small word before what follows: you sit on (or in) a chair, and you pass someone the salt that's on the table.",
        &[("I sat my chair.", "I sat on my chair."), ("Can you pass me salt?", "Can you pass me the salt?")],
    ),
    (
        "YourWelcome",
        "Your or you're?",
        "You're = you are. Your = belonging to you. \"You're welcome\" means \"you are welcome\".",
        &[("Your welcome.", "You're welcome.")],
    ),
    (
        "NounSubjectAgreement",
        "Making the verb match its subject",
        "The verb matches the main noun of the subject, not the nearest noun. In \"the results of the experiment\", the main noun is results (plural), so it's \"were\". Phrases like \"of the …\", \"in the hall\" or \"along with …\" don't change it. Each, one, everyone and neither take a singular verb.",
        &[
            ("The results of the experiment was clear.", "The results of the experiment were clear."),
            ("The boxes in the hall needs moving.", "The boxes in the hall need moving."),
            ("Each of the students have a book.", "Each of the students has a book."),
        ],
    ),
    (
        "AddressComma",
        "Commas around the person you're talking to",
        "When you speak to someone by name, commas separate the name from the rest of the sentence. Without them the meaning can change: \"Let's eat Grandma\" invites cannibalism; \"Let's eat, Grandma\" invites Grandma. Greetings get a comma after the name.",
        &[
            ("Let's eat Grandma.", "Let's eat, Grandma."),
            ("Thanks John for the update.", "Thanks, John, for the update."),
            ("Hi Marcus", "Hi Marcus,"),
        ],
    ),
    (
        "JoiningWords",
        "Joining words",
        "One joining word is enough to link two parts of a sentence. Although and but do the same job, as do because and so, so use one or the other. However can't join two full sentences with just a comma: use a semicolon (I tried; however, it failed) or start a new sentence.",
        &[
            ("Although it was late, but we went out.", "Although it was late, we went out."),
            ("I tried, however it failed.", "I tried; however, it failed."),
            ("Despite of the rain, we played.", "Despite the rain, we played."),
        ],
    ),
    (
        "WordOrder",
        "Word order",
        "A question asked inside a longer sentence uses normal statement order: \"Where is the office?\" but \"Can you tell me where the office is?\". Words like always, never and usually go after am/is/are: \"I am always late\".",
        &[
            ("Can you tell me where is the office?", "Can you tell me where the office is?"),
            ("I always am late.", "I am always late."),
        ],
    ),
    (
        "SoundAlike",
        "Words that sound alike",
        "English has many words that sound the same but mean different things (beer and bear, their and there, affect and effect). A spell-checker can't catch the swap because both are real words. Xenify looks at the words around it: \"spirits, bear and wine\" keeps the company of drinks, so beer was probably meant.",
        &[
            ("We stocked spirits, bear and wine.", "We stocked spirits, beer and wine."),
            ("The whether looks great today.", "The weather looks great today."),
        ],
    ),
    (
        "GreetingComma",
        "Comma after a greeting",
        "A comma after the greeting line (\"Hi Sam,\") is traditional in letters and emails. Many people now leave it out, so this is only suggested when your writing profile asks for optional commas.",
        &[("Hi Marcus\nThe report is attached.", "Hi Marcus,\nThe report is attached.")],
    ),
    (
        "Filler",
        "Words that add nothing",
        "Some words are habits of speech that make writing longer and less confident: \"just\" in requests (\"I just want…\"), a \"So\" at the start, and words like really, actually, basically and very. Leaving them out makes the point stronger.",
        &[
            ("I just want the report by Friday.", "I want the report by Friday."),
            ("So I want to move the meeting.", "I want to move the meeting."),
            ("It is really important.", "It is important."),
        ],
    ),
    (
        "ListColon",
        "Introduce a list with a colon",
        "When a sentence announces a list (\"There are five systems\") and then gives it, a colon separates the announcement from the items. With a comma, the list runs into the sentence and is hard to tell apart.",
        &[("There are 5 systems, ABC, CDE, EFG.", "There are five systems: ABC, CDE and EFG.")],
    ),
    (
        "VagueReference",
        "Unclear \"it\"",
        "When several things have just been named, \"it\" or \"this\" could mean any of them. Readers may guess wrong. Repeat the name instead.",
        &[(
            "ABC ingests data, CDE appends data, so it needs to be complete within 5 weeks.",
            "ABC ingests data and CDE appends it. CDE needs to be complete within five weeks.",
        )],
    ),
    (
        "NounStack",
        "Noun stacks",
        "Several nouns in a row (\"database migration rollback strategy document\") make the reader work out how they relate. Lead with the main thing and connect the rest with of, for or that.",
        &[(
            "Review the database migration rollback strategy document.",
            "Review the document describing the rollback strategy for the database migration.",
        )],
    ),
    (
        "LongLeadIn",
        "Main point first",
        "A long opening clause (because…, due to…, when…) makes readers hold details before they know why they matter. Start with the main point, then give the reason or condition.",
        &[(
            "Because the vendor changed the API contract last week without telling the integration team, we missed the deadline.",
            "We missed the deadline because the vendor changed the API contract last week without telling the integration team.",
        )],
    ),
    (
        "TooManyIdeas",
        "One idea per sentence",
        "This sentence joins many ideas with commas, and, which and brackets. Splitting it lets each idea land, and readers are less likely to lose the thread.",
        &[(
            "The service, which runs on the cluster, reads the queue, and it writes the results, but it retries on failure.",
            "The service runs on the cluster. It reads the queue and writes the results. On failure, it retries.",
        )],
    ),
    (
        "MainPointLate",
        "Put your request first",
        "In an email, readers decide from the first lines whether to act. If your question or request comes after several sentences of background, put it first and the background after.",
        &[(
            "We moved the service last week. It went well. Some jobs were slow. Could you approve the change?",
            "Could you approve the change? Background: we moved the service last week; it went well, though some jobs were slow.",
        )],
    ),
    (
        "UndefinedAcronym",
        "Spell out acronyms",
        "Not every reader knows every acronym. The first time you use one, write it in full with the short form in brackets; after that the short form is fine.",
        &[("Please approve the SLA.", "Please approve the service level agreement (SLA).")],
    ),
    (
        "ConsistentSpelling",
        "Consistent spelling",
        "Words like organise/organize can be spelled with -ise or -ize (UK English allows both). Either is fine, but one document should use one style throughout.",
        &[(
            "We organised the trip and will organize the next one.",
            "We organised the trip and will organise the next one.",
        )],
    ),
    (
        "ConsistentHyphens",
        "Consistent hyphens",
        "This word is written with a hyphen in one place and without one in another (email and e-mail). Pick one form and use it throughout.",
        &[("Send an e-mail. Reply to the email.", "Send an email. Reply to the email.")],
    ),
    (
        "ConsistentCapitals",
        "Consistent capitals",
        "This word has a capital letter in one place and not in another (Internet and internet). Pick one and use it throughout.",
        &[(
            "The Internet was slow, so the internet café closed.",
            "The internet was slow, so the internet café closed.",
        )],
    ),
    (
        "ConsistentNumbers",
        "Consistent numbers",
        "Small numbers are written in words in some places and as figures in others. Many style guides spell out one to nine; whichever you choose, use it throughout. Figures are always fine with units and times (3 kg, 5 pm).",
        &[("It took three days and 4 nights.", "It took three days and four nights.")],
    ),
    (
        "Cliche",
        "Cliché",
        "A phrase used so often that readers skim past it. A plain word usually says the same thing more directly.",
        &[("At the end of the day, we need low-hanging fruit.", "In the end, we need easy wins.")],
    ),
    (
        "WeakIntensifier",
        "One stronger word",
        "\"Very\" or \"really\" plus a plain word can often be one stronger word.",
        &[("The results were very good.", "The results were excellent.")],
    ),
    (
        "RepeatedOpener",
        "Varied sentence openings",
        "Several sentences in a row start with the same word. Varying how sentences begin makes text flow better.",
        &[("Then we ate. Then we slept. Then we left.", "We ate, then slept. In the morning, we left.")],
    ),
    (
        "PassiveVoice",
        "Passive voice",
        "In the passive voice the doer is hidden or comes last (\"the report was written by the team\"). Active sentences (\"the team wrote the report\") are usually shorter and clearer. Passive is fine when the doer doesn't matter; some profiles turn this check off.",
        &[("The decision was made by the board.", "The board made the decision.")],
    ),
    (
        "WordyPhrase",
        "Wordy phrase",
        "Some phrases use several words where one will do. Shorter is usually clearer.",
        &[("in order to", "to"), ("at this point in time", "now"), ("due to the fact that", "because")],
    ),
    (
        "LongSentence",
        "Long sentence",
        "Long sentences are harder to follow. Try splitting at a natural break (and, but, which) or cutting extra words. Your writing profile sets what counts as long.",
        &[],
    ),
    (
        "VeryLongSentence",
        "Very long sentence",
        "This sentence is very long and readers may lose the thread. Try splitting it into two or three sentences.",
        &[],
    ),
    (
        "Adverb",
        "Adverbs",
        "Words ending in -ly can often be cut or replaced by a stronger verb: \"ran quickly\" → \"raced\".",
        &[("She ran very quickly.", "She raced.")],
    ),
];

/// The error codes (`rules/codes.json`): code → (explanation, how suggestions are found).
fn codes() -> &'static std::collections::HashMap<String, (Explanation, String)> {
    static C: std::sync::OnceLock<std::collections::HashMap<String, (Explanation, String)>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../rules/codes.json")).expect("codes.json parses");
        v.as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, e)| {
                let examples = e["examples"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string())))
                    .collect();
                let ex = Explanation {
                    title: e["title"].as_str().unwrap_or(k).to_string(),
                    text: e["text"].as_str().unwrap_or("").to_string(),
                    examples,
                };
                (k.clone(), (ex, e["candidates"].as_str().unwrap_or("none").to_string()))
            })
            .collect()
    })
}

/// The explanation for an error code (`article_missing`, `comma_splice`…).
pub fn explain_code(code: &str) -> Option<Explanation> {
    codes().get(code).map(|(e, _)| e.clone())
}

/// How suggestions for an error code are found (see `rules/codes.json`):
/// `insert:a,b`, `replace:a,b`, `soundalikes`, `verbforms`, `pronouns`,
/// `commas`, `splice`, `hyphen`, `apostrophes`, `delete` or `none`.
pub fn code_candidates(code: &str) -> &'static str {
    codes().get(code).map(|(_, c)| c.as_str()).unwrap_or("none")
}

/// Every error code with an explanation.
pub fn error_codes() -> Vec<&'static str> {
    let mut v: Vec<&str> = codes().keys().map(String::as_str).collect();
    v.sort();
    v
}

/// The explanation for `rule` (Xenify's own rules, style checks, register
/// features and `Spotter:<code>`). `None` for Harper's rules: use their description.
pub fn explain(rule: &str) -> Option<Explanation> {
    if let Some(code) = rule.strip_prefix("Spotter:") {
        return explain_code(code);
    }
    if let Some((_, title, text, ex)) = OWN.iter().find(|(r, ..)| *r == rule) {
        return Some(Explanation {
            title: title.to_string(),
            text: text.to_string(),
            examples: ex.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        });
    }
    if let Some(name) = rule.strip_prefix("Register:") {
        let f = Feature::ALL.into_iter().find(|f| format!("{f:?}") == name)?;
        return Some(Explanation {
            title: f.name().to_string(),
            text: format!(
                "{}. This isn't a mistake: it's how many people speak and write in casual settings or in their own variety of English. Your writing profile decides whether to allow it, note it or flag it — change it in Proofing › Edit profiles, or choose \"Allow\" from this menu.",
                f.describe()
            ),
            examples: Vec::new(),
        });
    }
    let (_, src) = crate::rules::WEIR_RULES.iter().find(|(n, _)| *n == rule)?;
    Some(weir_explanation(src))
}

/// From a Weir rule: its `# why:` comment (or description) and `test` lines.
fn weir_explanation(src: &str) -> Explanation {
    let field = |key: &str| {
        src.lines().find_map(|l| {
            l.strip_prefix(&format!("let {key} ")).map(|v| v.trim().trim_matches('"').replace("\\\"", "\""))
        })
    };
    let why: Vec<&str> = src.lines().filter_map(|l| l.strip_prefix("# why: ")).collect();
    let text = if why.is_empty() { field("description").unwrap_or_default() } else { why.join(" ") };
    let examples = src
        .lines()
        .filter_map(|l| l.strip_prefix("test "))
        .filter_map(|rest| {
            let parts: Vec<&str> = rest.split('"').collect();
            (parts.len() >= 4 && parts[1] != parts[3]).then(|| (parts[1].to_string(), parts[3].to_string()))
        })
        .take(3)
        .collect();
    Explanation { title: field("message").unwrap_or_default().replace('`', ""), text: text.replace('`', ""), examples }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_own_rule_is_explained() {
        for name in crate::own_rules() {
            let e = explain(name).unwrap_or_else(|| panic!("{name} has no explanation"));
            assert!(!e.text.is_empty(), "{name}");
            assert!(!e.examples.is_empty(), "{name} has no examples");
        }
        for f in Feature::ALL {
            assert!(explain(&format!("Register:{f:?}")).is_some());
        }
        assert!(error_codes().len() >= 27);
        for c in error_codes() {
            let e = explain_code(c).unwrap();
            assert!(!e.text.is_empty() && !e.examples.is_empty(), "{c}");
        }
        assert_eq!(code_candidates("article_missing"), "insert:the,a,an");
    }
}
