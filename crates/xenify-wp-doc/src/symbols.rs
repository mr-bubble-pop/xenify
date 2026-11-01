//! Characters for Insert › Symbols & emoji, by group, with names to search by.

/// Symbols by group: each line is a group name, then "character name" pairs.
pub const SYMBOLS: &str = "\
Common|© copyright|® registered|™ trade mark|§ section|¶ paragraph|† dagger|‡ double dagger|• bullet|· middle dot|… ellipsis|– en dash|— em dash|‘ left single quote|’ right single quote apostrophe|“ left double quote|” right double quote|« left guillemet|» right guillemet|° degree|′ prime minute|″ double prime second|№ numero|✓ check tick|✗ cross ballot|★ star black|☆ star white
Arrows|← left arrow|→ right arrow|↑ up arrow|↓ down arrow|↔ left right arrow|↕ up down arrow|⇐ left double arrow|⇒ right double arrow implies|⇑ up double arrow|⇓ down double arrow|⇔ left right double arrow iff|↩ return arrow|↪ hook arrow|⟶ long right arrow
Maths|± plus minus|× times multiply|÷ divide|≠ not equal|≈ approximately|≡ identical|≤ less or equal|≥ greater or equal|∞ infinity|√ square root|∑ sum sigma|∏ product|∫ integral|∂ partial|∆ delta increment|∇ nabla|∈ element of|∉ not element of|⊂ subset|⊃ superset|∪ union|∩ intersection|∧ and|∨ or|¬ not|∀ for all|∃ exists|∅ empty set|∝ proportional|∠ angle|⊥ perpendicular|∴ therefore|∵ because|‰ per mille|½ one half|⅓ one third|⅔ two thirds|¼ one quarter|¾ three quarters|⅛ one eighth|¹ superscript one|² superscript two squared|³ superscript three cubed|ⁿ superscript n
Greek|α alpha|β beta|γ gamma|δ delta|ε epsilon|ζ zeta|η eta|θ theta|ι iota|κ kappa|λ lambda|μ mu micro|ν nu|ξ xi|ο omicron|π pi|ρ rho|σ sigma|τ tau|υ upsilon|φ phi|χ chi|ψ psi|ω omega|Γ capital gamma|Δ capital delta|Θ capital theta|Λ capital lambda|Ξ capital xi|Π capital pi|Σ capital sigma|Φ capital phi|Ψ capital psi|Ω capital omega ohm
Currency|$ dollar|€ euro|£ pound|¥ yen yuan|¢ cent|₹ rupee|₽ rouble|₩ won|₪ shekel|₫ dong|₺ lira|₿ bitcoin|¤ currency
Punctuation|¿ inverted question|¡ inverted exclamation|‽ interrobang|※ reference mark|⁂ asterism|‖ double bar|¦ broken bar|‹ single left guillemet|› single right guillemet|„ low double quote|‚ low single quote
Shapes|■ black square|□ white square|▪ small black square|▫ small white square|● black circle|○ white circle|◆ black diamond|◇ white diamond|▲ black up triangle|△ white up triangle|▼ black down triangle|▽ white down triangle|♠ spade|♣ club|♥ heart|♦ diamond suit|☐ ballot box|☑ ballot box with check|☒ ballot box with x";

/// The groups: (group name, [(character, name)]).
pub fn groups() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    SYMBOLS
        .lines()
        .map(|line| {
            let mut parts = line.split('|');
            let group = parts.next().unwrap_or_default();
            let items = parts.filter_map(|p| p.split_once(' ')).collect();
            (group, items)
        })
        .collect()
}
