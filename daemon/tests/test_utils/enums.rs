// standard crates
use std::fmt::Debug;

// external crates
use serde::Serialize;

/// Checks that `cases` lists every variant in order, that each one converts to
/// its expected value, and that both sides serialize to the same wire string.
pub fn assert_converts_each<D, W>(variants: Vec<D>, cases: &[(D, W)])
where
    D: PartialEq + Debug + Serialize,
    W: PartialEq + Debug + Serialize,
    for<'a> W: From<&'a D>,
{
    let tested: Vec<&D> = cases.iter().map(|(domain, _)| domain).collect();
    assert_eq!(tested, variants.iter().collect::<Vec<_>>());
    for (domain, wire) in cases {
        assert_eq!(&W::from(domain), wire);
        let wire_json = serde_json::to_value(wire).unwrap();
        assert_eq!(wire_json, serde_json::to_value(domain).unwrap());
    }
}
