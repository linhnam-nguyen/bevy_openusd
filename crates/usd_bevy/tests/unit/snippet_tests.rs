use super::*;

#[test]
fn valid_snippet_parses_and_opens() {
    let s = UsdSnippet::new("#usda 1.0\ndef Xform \"Foo\"\n{\n    custom double x = 2\n}\n");
    assert!(s.parse().is_ok(), "well-formed usda parses");
    let stage = s.open_stage().expect("opens as a stage");
    assert!(
        stage
            .prim(openusd::sdf::path("/Foo").unwrap())
            .is_valid()
            .unwrap_or(false),
        "the prim exists on the opened stage"
    );
}

#[test]
fn malformed_snippet_fails_to_parse() {
    // This is the same validator the `usd!` macro runs at compile time; a
    // broken body here is a compile error there.
    let s = UsdSnippet::new("#usda 1.0\ndef Xform \"Foo\" { this is not usda ]]");
    assert!(s.parse().is_err(), "malformed usda is rejected");
}

#[test]
fn macro_escapes_braces_and_interpolates_multiple() {
    let a = 1_i32;
    let b = 2_i32;
    let s = crate::usd!(
        "#usda 1.0\n\
         def Scope \"S\"\n\
         {\n\
             int x = ${a}\n\
             int y = ${b}\n\
         }\n"
    );
    let t = s.text();
    assert!(t.contains("int x = 1"), "first interpolation: {t}");
    assert!(t.contains("int y = 2"), "second interpolation: {t}");
    assert!(
        t.contains('{') && t.contains('}'),
        "literal usda braces survive format escaping: {t}"
    );
    assert!(s.parse().is_ok(), "interpolated result is valid usda");
}

#[test]
fn macro_static_only() {
    let s = crate::usd!("#usda 1.0\ndef Scope \"S\"\n{\n}\n");
    assert!(s.parse().is_ok());
    assert!(s.text().contains("def Scope"));
}

#[test]
fn macro_string_interpolation_inside_quotes() {
    let name = "Widget";
    let s = crate::usd!("#usda 1.0\ndef Xform \"${name}\"\n{\n}\n");
    assert!(
        s.text().contains("\"Widget\""),
        "interpolation inside quotes: {}",
        s.text()
    );
    assert!(s.parse().is_ok());
}
