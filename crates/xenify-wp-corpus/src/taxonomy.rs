//! What the corpus covers: kinds of mistake, writing settings and informal
//! registers. Descriptions and examples are our own wording; the list of
//! kinds draws on common-error research (see `docs` in each entry).

/// A kind of mistake the checker should find.
pub struct ErrorType {
    pub id: &'static str,
    /// What to put in the sentences (for the generator).
    pub describe: &'static str,
    /// Our own example: wrong → right.
    pub example: (&'static str, &'static str),
    /// Settings it is limited to (empty: any).
    pub only: &'static [&'static str],
}

pub const ERROR_TYPES: &[ErrorType] = &[
    ErrorType {
        id: "subject_verb_agreement",
        describe: "a verb that does not agree with its noun subject, including subjects followed by 'of …' phrases",
        example: ("The boxes in the hall needs moving.", "The boxes in the hall need moving."),
        only: &[],
    },
    ErrorType {
        id: "pronoun_verb_agreement",
        describe: "a verb that does not agree with a pronoun subject (we was, he don't, I has, this are)",
        example: ("We was waiting outside.", "We were waiting outside."),
        only: &[],
    },
    ErrorType {
        id: "verb_form",
        describe: "a wrong verb form: past tense vs past participle (I seen, has went, had ate), base form after he/she",
        example: ("I seen the report yesterday.", "I saw the report yesterday."),
        only: &[],
    },
    ErrorType {
        id: "pronoun_case",
        describe: "subject vs object pronoun mistakes (me and him went, between you and I, her and me are, whom for who)",
        example: ("Me and Sam finished early.", "Sam and I finished early."),
        only: &[],
    },
    ErrorType {
        id: "its_its",
        describe: "its vs it's confusion in either direction",
        example: ("Its going to rain.", "It's going to rain."),
        only: &[],
    },
    ErrorType {
        id: "homophone",
        describe: "a sound-alike word swap: their/there/they're, your/you're, to/too/two, then/than, lose/loose, whose/who's, were/where/we're, passed/past, weather/whether",
        example: ("I'd rather walk then drive.", "I'd rather walk than drive."),
        only: &[],
    },
    ErrorType {
        id: "confused_words",
        describe: "commonly confused words: affect/effect, complement/compliment, principal/principle, accept/except, lie/lay, farther/further, elicit/illicit, desert/dessert, stationary/stationery, everyday/every day, altogether/all together, already/all ready, advice/advise, breath/breathe",
        example: ("The new policy will effect everyone.", "The new policy will affect everyone."),
        only: &[],
    },
    ErrorType {
        id: "real_word_typo",
        describe: "an embarrassing typo that is itself a real word, so a spell-checker misses it: pubic for public, manger for manager, prostrate for prostate, defiantly for definitely, asses for assess, shit for shift, untied for united, barley for barely, form for from, now for not, existing for exiting, sing for sign, lost for lots",
        example: ("Please contact the pubic safety office.", "Please contact the public safety office."),
        only: &[],
    },
    ErrorType {
        id: "eggcorn",
        describe: "a misheard fixed phrase: should of/could of, for all intensive purposes, nip it in the butt, free reign, peak my interest, wet your appetite, ripe with, one in the same, case and point, on accident",
        example: ("That should peak their interest.", "That should pique their interest."),
        only: &[],
    },
    ErrorType {
        id: "article_missing",
        describe: "a missing a/an/the before a singular countable noun (around corner, I have pet dog, she is teacher)",
        example: ("We met at station around noon.", "We met at the station around noon."),
        only: &[],
    },
    ErrorType {
        id: "article_wrong",
        describe: "a wrong article: a/an before vowel sounds (a hour, an university), the vs a, an extra article",
        example: ("It took a hour to fix.", "It took an hour to fix."),
        only: &[],
    },
    ErrorType {
        id: "preposition",
        describe: "a wrong or missing preposition: on/in/at with times and places, since vs for with durations, listen music, depend of, married with, arrive to",
        example: ("I'll call you on 10 minutes.", "I'll call you in 10 minutes."),
        only: &[],
    },
    ErrorType {
        id: "verb_pattern",
        describe: "a wrong verb pattern: look forward to hear, explained me the problem, suggest him to go, chose start, let him to go, make him to",
        example: ("I look forward to hear from you.", "I look forward to hearing from you."),
        only: &[],
    },
    ErrorType {
        id: "plural_form",
        describe: "a wrong plural or countability: childs, informations, advices, peoples, much people, less mistakes, two sheeps",
        example: ("We need more informations about the childs.", "We need more information about the children."),
        only: &[],
    },
    ErrorType {
        id: "comparative",
        describe: "a doubled or wrong comparative or superlative: more taller, most easiest, gooder, more better",
        example: ("This one is more cheaper.", "This one is cheaper."),
        only: &[],
    },
    ErrorType {
        id: "missing_word",
        describe: "a small word left out (I going home, she been there, need to the files)",
        example: ("We need finish the draft.", "We need to finish the draft."),
        only: &[],
    },
    ErrorType {
        id: "repeated_word",
        describe: "a word typed twice, including across a line break or with different case (the the, to to, I I)",
        example: ("Send it to the the team.", "Send it to the team."),
        only: &[],
    },
    ErrorType {
        id: "word_order",
        describe: "wrong word order (always is late, I know what is it, a red big car)",
        example: ("Can you tell me where is the office?", "Can you tell me where the office is?"),
        only: &[],
    },
    ErrorType {
        id: "comma_vocative",
        describe: "a missing comma before or after a person being addressed, which changes the meaning (Let's eat Grandma, Thanks John)",
        example: ("Let's eat Grandma.", "Let's eat, Grandma."),
        only: &[],
    },
    ErrorType {
        id: "comma_intro",
        describe: "a missing comma after an introductory word, phrase or clause (However we…, If you can come let me know)",
        example: ("If you can come let me know.", "If you can come, let me know."),
        only: &[],
    },
    ErrorType {
        id: "comma_list_meaning",
        describe: "missing list commas that change the meaning (I love cooking my family and my pets)",
        example: ("She enjoys cooking her kids and her dog.", "She enjoys cooking, her kids and her dog."),
        only: &[],
    },
    ErrorType {
        id: "comma_splice",
        describe: "two full sentences joined with only a comma, or with nothing (run-on)",
        example: ("The meeting ran late, we missed the train.", "The meeting ran late, so we missed the train."),
        only: &[],
    },
    ErrorType {
        id: "apostrophe",
        describe: "a wrong apostrophe: plurals with apostrophes (apple's for sale, the Smith's), missing possessive apostrophes (the teams results), wrong place (childrens')",
        example: ("Fresh apple's for sale.", "Fresh apples for sale."),
        only: &[],
    },
    ErrorType {
        id: "capitalization",
        describe: "wrong capitals: lowercase i, sentence starting lowercase, lowercase proper nouns or days, random capitals on common nouns",
        example: ("i told them on monday.", "I told them on Monday."),
        only: &[],
    },
    ErrorType {
        id: "hyphen",
        describe: "missing or wrong hyphens: follow-up vs follow up, well-known before a noun, log in vs login, set up vs setup",
        example: ("Let's schedule a follow up meeting.", "Let's schedule a follow-up meeting."),
        only: &[],
    },
    ErrorType {
        id: "fragment",
        describe: "a sentence fragment written as a full sentence (Because the server was down.)",
        example: ("We stopped. Because the server was down.", "We stopped because the server was down."),
        only: &[],
    },
    ErrorType {
        id: "tense_shift",
        describe: "an unnecessary switch of tense inside a sentence or between close sentences",
        example: ("She walks in and said hello.", "She walked in and said hello."),
        only: &[],
    },
    ErrorType {
        id: "pronoun_antecedent",
        describe: "a pronoun that does not agree with what it refers to (Each student must bring their… is fine; The company lost their way is a mistake in formal writing; a team… they… it)",
        example: ("The committee changed their mind on its report.", "The committee changed its mind on its report."),
        only: &["academic_paper", "workplace_email", "formal_letter", "news_article", "technical_docs"],
    },
    ErrorType {
        id: "sign_off",
        describe: "a misspelled or mangled email greeting or sign-off (Kind regrads, Best wishs, Sincerly, Your welcome, Thanks you), or a missing comma after the greeting",
        example: ("Kind regrads,", "Kind regards,"),
        only: &["workplace_email", "formal_letter", "cover_letter"],
    },
    ErrorType {
        id: "spelling",
        describe: "a common misspelling that is not a real word (recieve, seperate, definately, occured, untill, accomodate, wich, alot)",
        example: ("I definately recieved it.", "I definitely received it."),
        only: &[],
    },
];

/// Where the text is from (shapes vocabulary and tone).
pub struct Setting {
    pub id: &'static str,
    pub describe: &'static str,
    /// Informal registers that are natural here.
    pub informal: bool,
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        id: "academic_paper",
        describe: "an academic paper (abstract, methods or discussion section)",
        informal: false,
    },
    Setting { id: "school_essay", describe: "a high-school or first-year college essay", informal: false },
    Setting { id: "workplace_email", describe: "an email to colleagues or a client at work", informal: false },
    Setting { id: "formal_letter", describe: "a formal or legal letter", informal: false },
    Setting { id: "technical_docs", describe: "software or product documentation", informal: false },
    Setting { id: "news_article", describe: "a local news article", informal: false },
    Setting { id: "marketing_copy", describe: "marketing copy for a website or flyer", informal: false },
    Setting { id: "cover_letter", describe: "a job cover letter or CV summary", informal: false },
    Setting { id: "professional_social", describe: "a post on a professional networking site", informal: false },
    Setting {
        id: "public_notice",
        describe: "a sign, notice or announcement in a shop, school or public building",
        informal: false,
    },
    Setting { id: "social_post", describe: "a personal social-media post", informal: true },
    Setting { id: "video_comments", describe: "comments under a short-video or video-sharing post", informal: true },
    Setting { id: "group_chat", describe: "messages in a group chat between friends", informal: true },
    Setting { id: "forum_post", describe: "a post on a hobby forum", informal: true },
    Setting { id: "product_review", describe: "an online product or restaurant review", informal: true },
    Setting { id: "fiction", describe: "fiction narrative, possibly with dialogue", informal: true },
];

/// An informal register or feature of writing (correct in its own terms).
pub struct Register {
    pub id: &'static str,
    pub describe: &'static str,
}

pub const REGISTERS: &[Register] = &[
    Register { id: "standard", describe: "correct standard written English suited to the setting" },
    Register { id: "slang", describe: "current casual slang (no grammar mistakes otherwise)" },
    Register { id: "texting", describe: "texting shorthand and abbreviations (u, rn, idk, tbh, lol)" },
    Register {
        id: "spoken_spelling",
        describe: "spellings that follow speech (gonna, wanna, gotta, kinda, finna, dat, nothin')",
    },
    Register {
        id: "aae",
        describe: "African American English used naturally and respectfully by a fluent speaker: habitual be, zero copula, completive done, double negatives, ain't — consistent with its own grammar, not a caricature",
    },
    Register {
        id: "regional",
        describe: "regional dialect features used naturally (Southern US y'all and might could, Scottish wee and dinnae, Irish 'I'm after doing', British 'we was' in casual speech, Australian slang)",
    },
    Register { id: "profanity", describe: "casual swearing" },
    Register { id: "emoji_exclaim", describe: "emoji, smileys, kisses (xx) and multiple exclamation marks" },
    Register {
        id: "contractions_first_person",
        describe: "contractions and first-person voice (fine in most writing, discouraged in some formal writing)",
    },
    Register { id: "hedging_filler", describe: "hedges and fillers (like, basically, sort of, I guess, just)" },
];

pub fn error_type(id: &str) -> Option<&'static ErrorType> {
    ERROR_TYPES.iter().find(|e| e.id == id)
}

pub fn setting(id: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.id == id)
}

pub fn register(id: &str) -> Option<&'static Register> {
    REGISTERS.iter().find(|r| r.id == id)
}
