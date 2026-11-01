//! Xenify: numbers are exact decimals (see crate::num).

use crate::test::util::new_empty_model;

#[test]
fn long_numbers_keep_every_digit() {
    let mut model = new_empty_model();
    model._set("A1", "12345678901234567890");
    model._set("A2", "=A1+1");
    model._set("A3", "=A1=12345678901234567891");
    model._set("A4", "=12345678901234567890+0.5");
    model._set("A5", "0.1");
    model._set("A6", "=A5+0.2=0.3");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "12345678901234567890");
    assert_eq!(model._get_text("A2"), "12345678901234567891");
    assert_eq!(model._get_text("A3"), "FALSE");
    assert_eq!(model._get_text("A4"), "12345678901234567890.5");
    assert_eq!(model._get_text("A6"), "TRUE");
    // What editing starts from, and the formula as written, keep every digit.
    assert_eq!(model.get_localized_cell_content(0, 1, 1).unwrap(), "12345678901234567890");
    assert_eq!(model.get_localized_cell_content(0, 4, 1).unwrap(), "=12345678901234567890+0.5");
    // Typing the input back in changes nothing.
    let input = model.get_localized_cell_content(0, 1, 1).unwrap();
    model._set("A1", &input);
    model.evaluate();
    assert_eq!(model._get_text("A2"), "12345678901234567891");
}

#[test]
fn floating_point_reads_as_before() {
    let mut model = new_empty_model();
    model._set("A1", "=SQRT(2)");
    model._set("A2", "=0.1*SQRT(4)/2");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1.414213562");
    assert_eq!(model._get_text("A2"), "0.1");
}
