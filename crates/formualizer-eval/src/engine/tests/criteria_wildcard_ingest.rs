use super::common::arrow_eval_config;
use crate::engine::Engine;
use crate::test_workbook::TestWorkbook;
use crate::traits::EvaluationContext;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::ReferenceType;

#[test]
fn wildcard_masks_preserve_scalar_contract_for_base_and_overlay() {
    for overlay in [false, true] {
        for value in [
            LiteralValue::Number(123.0),
            LiteralValue::Boolean(true),
            LiteralValue::Empty,
        ] {
            let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
            {
                let mut ingest = engine.begin_bulk_ingest_arrow();
                ingest.add_sheet("S", 1, 3);
                for row in 0..9 {
                    ingest
                        .append_row(
                            "S",
                            &[if row == 4 && !overlay {
                                value.clone()
                            } else {
                                LiteralValue::Text("abc".into())
                            }],
                        )
                        .unwrap();
                }
                ingest.finish().unwrap();
            }
            let range = ReferenceType::range(Some("S".into()), Some(2), Some(1), Some(8), Some(1));
            let pred = crate::args::parse_criteria(&LiteralValue::Text("*".into())).unwrap();
            // "*" matches text only; the one non-text cell is not counted.
            if overlay {
                let view = engine.resolve_range_view(&range, "S").unwrap();
                assert_eq!(
                    engine
                        .build_criteria_mask(&view, 0, &pred)
                        .unwrap()
                        .true_count(),
                    7
                );
                engine.set_cell_value("S", 5, 1, value.clone()).unwrap();
            }
            let view = engine.resolve_range_view(&range, "S").unwrap();
            assert_eq!(
                engine
                    .build_criteria_mask(&view, 0, &pred)
                    .unwrap()
                    .true_count(),
                6,
                "mixed data must retain a cacheable scalar-equivalent mask"
            );
            for pattern in ["1*", "?", "TRUE*", "~*"] {
                let pred =
                    crate::args::parse_criteria(&LiteralValue::Text(pattern.into())).unwrap();
                let mask = engine.build_criteria_mask(&view, 0, &pred).unwrap();
                for row in 0..7 {
                    assert_eq!(
                        mask.value(row),
                        crate::builtins::criteria_match(&pred, &view.get_cell(row, 0)),
                        "{pattern} row={row}"
                    );
                }
            }
        }
    }
}

#[test]
fn ne_and_eq_wildcard_criteria_in_if_functions() {
    // A1:A5 = In Approval, In Progress, In Review, 5, (blank); B1:B5 = 30, 20, 10, 40, 50.
    let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
    let rows: [(LiteralValue, f64); 5] = [
        (LiteralValue::Text("In Approval".into()), 30.0),
        (LiteralValue::Text("In Progress".into()), 20.0),
        (LiteralValue::Text("In Review".into()), 10.0),
        (LiteralValue::Number(5.0), 40.0),
        (LiteralValue::Empty, 50.0),
    ];
    for (row, (tag, amount)) in rows.into_iter().enumerate() {
        let row = row as u32 + 1;
        if tag != LiteralValue::Empty {
            engine.set_cell_value("Sheet1", row, 1, tag).unwrap();
        }
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(amount))
            .unwrap();
    }
    for (formula, expected) in [
        ("=COUNTIF(A1:A3,\"<>*approval\")", 2.0),
        ("=COUNTIF(A1:A5,\"<>*approval\")", 4.0),
        ("=COUNTIFS(A1:A3,\"<>In*\")", 0.0),
        ("=COUNTIF(A1:A5,\"<>*\")", 2.0),
        ("=COUNTIF(A1:A5,\"=*approval\")", 1.0),
        ("=SUMIF(A1:A3,\"<>*approval\",B1:B3)", 30.0),
        ("=AVERAGEIFS(B1:B3,A1:A3,\"<>*approval\")", 15.0),
        ("=SUMIFS(B1:B5,A1:A5,\"<>*re*\",B1:B5,\">=20\")", 120.0),
        // Without a wildcard "<>" is a whole-value comparison.
        ("=COUNTIF(A1:A3,\"<>in approval\")", 2.0),
    ] {
        engine
            .set_cell_formula(
                "Sheet1",
                1,
                10,
                formualizer_parse::parser::parse(formula).unwrap(),
            )
            .unwrap();
        engine.evaluate_cell("Sheet1", 1, 10).unwrap();
        assert_eq!(
            engine.get_cell_value("Sheet1", 1, 10).unwrap(),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}

#[test]
fn text_column_masks_read_escapes_and_like_metacharacters_literally() {
    // A text-only column takes the Arrow LIKE kernel: Excel's `~` escape and
    // text holding LIKE's own `%`, `_` or `\` must match as the scalar
    // matcher does, through the mask and through COUNTIF.
    let cells = [
        "a*b", "axb", "a_b", "a~b", "ab", "50%", "50x", "c\\d", "cd", "?", "x",
    ];
    let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
    {
        let mut ingest = engine.begin_bulk_ingest_arrow();
        ingest.add_sheet("S", 1, 4);
        for cell in cells {
            ingest
                .append_row("S", &[LiteralValue::Text(cell.into())])
                .unwrap();
        }
        ingest.finish().unwrap();
    }
    let last = cells.len() as u32;
    let range = ReferenceType::range(Some("S".into()), Some(1), Some(1), Some(last), Some(1));
    for (criterion, matched) in [
        ("a~*b", &["a*b"][..]),
        ("=a~*b", &["a*b"]),
        ("a~~b", &["a~b"]),
        ("a_b", &["a_b"]),
        (
            "<>a_b",
            &[
                "a*b", "axb", "a~b", "ab", "50%", "50x", "c\\d", "cd", "?", "x",
            ],
        ),
        ("a?b", &["a*b", "axb", "a_b", "a~b"]),
        ("*%", &["50%"]),
        ("c\\d", &["c\\d"]),
        ("c\\*", &["c\\d"]),
        ("~?", &["?"]),
        (
            "<>~?",
            &[
                "a*b", "axb", "a_b", "a~b", "ab", "50%", "50x", "c\\d", "cd", "x",
            ],
        ),
    ] {
        let pred = crate::args::parse_criteria(&LiteralValue::Text(criterion.into())).unwrap();
        let view = engine.resolve_range_view(&range, "S").unwrap();
        let mask = engine.build_criteria_mask(&view, 0, &pred);
        for (row, cell) in cells.iter().enumerate() {
            let expected = matched.contains(cell);
            assert_eq!(
                crate::builtins::criteria_match(&pred, &view.get_cell(row, 0)),
                expected,
                "{criterion} on {cell}"
            );
            if let Some(mask) = &mask {
                assert_eq!(mask.value(row), expected, "mask {criterion} on {cell}");
            }
        }
        let formula = format!("=COUNTIF(A1:A{last},\"{criterion}\")");
        engine
            .set_cell_formula(
                "S",
                1,
                3,
                formualizer_parse::parser::parse(&formula).unwrap(),
            )
            .unwrap();
        engine.evaluate_cell("S", 1, 3).unwrap();
        assert_eq!(
            engine.get_cell_value("S", 1, 3).unwrap(),
            LiteralValue::Number(matched.len() as f64),
            "{formula}"
        );
    }
}
