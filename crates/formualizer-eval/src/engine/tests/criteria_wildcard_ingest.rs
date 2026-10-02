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
        ("a~~?", &["a~b"]),
        // Without "*" or "?" a tilde is an ordinary character.
        ("a~b", &["a~b"]),
        (
            "<>a~b",
            &[
                "a*b", "axb", "a_b", "ab", "50%", "50x", "c\\d", "cd", "?", "x",
            ],
        ),
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

/// Runs `formula` in S!C1 and returns its value.
fn eval_in_c1(engine: &mut Engine<TestWorkbook>, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula(
            "S",
            1,
            3,
            formualizer_parse::parser::parse(formula).unwrap(),
        )
        .unwrap();
    engine.evaluate_cell("S", 1, 3).unwrap();
    engine.get_cell_value("S", 1, 3).unwrap()
}

fn mask_bit(mask: &arrow_array::BooleanArray, row: usize) -> bool {
    use arrow_array::Array as _;
    mask.is_valid(row) && mask.value(row)
}

#[test]
fn temporal_overlay_values_never_match_text_patterns() {
    // A date or duration set over a text column lands in the delta overlay as
    // a serial (45306, 1.5), whose lowered text lane spells it out. Text
    // wildcards match text only, so "*", "=*", "4*" and "1*" skip those cells
    // and "<>*" keeps them, through the mask and through COUNTIF. Two edits
    // in a 200-row chunk stay in the overlay (no compaction into the base).
    const ROWS: u32 = 200;
    let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
    {
        let mut ingest = engine.begin_bulk_ingest_arrow();
        ingest.add_sheet("S", 1, 256);
        for _ in 0..ROWS {
            ingest
                .append_row("S", &[LiteralValue::Text("abc".into())])
                .unwrap();
        }
        ingest.finish().unwrap();
    }
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    engine
        .set_cell_value("S", 3, 1, LiteralValue::Date(date))
        .unwrap();
    engine
        .set_cell_value(
            "S",
            4,
            1,
            LiteralValue::Duration(chrono::Duration::hours(36)),
        )
        .unwrap();
    let range = ReferenceType::range(Some("S".into()), Some(1), Some(1), Some(ROWS), Some(1));
    let text_rows = f64::from(ROWS - 2);
    for (criterion, expected) in [
        ("*", text_rows),
        ("=*", text_rows),
        ("4*", 0.0),
        ("1*", 0.0),
        ("<>*", 2.0),
    ] {
        let pred = crate::args::parse_criteria(&LiteralValue::Text(criterion.into())).unwrap();
        let view = engine.resolve_range_view(&range, "S").unwrap();
        let mask = engine.build_criteria_mask(&view, 0, &pred).unwrap();
        for row in 0..ROWS as usize {
            assert_eq!(
                mask_bit(&mask, row),
                crate::builtins::criteria_match(&pred, &view.get_cell(row, 0)),
                "mask {criterion} row={row}"
            );
        }
        assert_eq!(
            eval_in_c1(
                &mut engine,
                &format!("=COUNTIF(A1:A{ROWS},\"{criterion}\")")
            ),
            LiteralValue::Number(expected),
            "COUNTIF {criterion}"
        );
    }
}

#[test]
fn wildcard_eq_and_ne_stay_complements_under_unicode_case() {
    // The text lanes and the patterns are folded as the per-cell matcher
    // folds them, so "=p" and "<>p" split every text cell between them, and
    // a non-text cell elsewhere in the range (which sends the column to the
    // per-cell mask) does not change the answer for a text cell.
    let cells = [
        "\u{17f}x", "sx", "Sx", "\u{3c2}", "\u{3c3}", "\u{3a3}", "\u{212a}", "k", "x",
    ];
    let n = cells.len();
    let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
    {
        let mut ingest = engine.begin_bulk_ingest_arrow();
        ingest.add_sheet("S", 1, 4);
        for cell in cells.iter().chain(["pad"].iter()) {
            ingest
                .append_row("S", &[LiteralValue::Text((*cell).into())])
                .unwrap();
        }
        ingest.finish().unwrap();
    }
    let range = ReferenceType::range(Some("S".into()), Some(1), Some(1), Some(n as u32), Some(1));
    let mixed_range = ReferenceType::range(
        Some("S".into()),
        Some(1),
        Some(1),
        Some(n as u32 + 1),
        Some(1),
    );
    let pairs = [
        ("=s?", "<>s?"),
        ("=*\u{3c3}*", "<>*\u{3c3}*"),
        ("\u{17f}?", "<>\u{17f}?"),
        ("sx", "<>sx"),
        ("k", "<>k"),
        ("\u{3c3}", "<>\u{3c3}"),
    ];
    let mut text_only = Vec::new();
    for (eq, ne) in pairs {
        let eq_pred = crate::args::parse_criteria(&LiteralValue::Text(eq.into())).unwrap();
        let ne_pred = crate::args::parse_criteria(&LiteralValue::Text(ne.into())).unwrap();
        let view = engine.resolve_range_view(&range, "S").unwrap();
        let eq_mask = engine.build_criteria_mask(&view, 0, &eq_pred).unwrap();
        let ne_mask = engine.build_criteria_mask(&view, 0, &ne_pred).unwrap();
        let mut bits = Vec::new();
        for (row, cell) in cells.iter().enumerate() {
            let value = view.get_cell(row, 0);
            let hit = mask_bit(&eq_mask, row);
            assert_eq!(
                hit,
                crate::builtins::criteria_match(&eq_pred, &value),
                "{eq} on {cell}"
            );
            assert_eq!(
                mask_bit(&ne_mask, row),
                !hit,
                "{ne} is not the complement of {eq} on {cell}"
            );
            assert_eq!(
                crate::builtins::criteria_match(&ne_pred, &value),
                !hit,
                "{ne} per cell on {cell}"
            );
            bits.push(hit);
        }
        let count = |engine: &mut Engine<TestWorkbook>, c: &str| match eval_in_c1(
            engine,
            &format!("=COUNTIF(A1:A{n},\"{c}\")"),
        ) {
            LiteralValue::Number(x) => x,
            other => panic!("COUNTIF {c}: {other:?}"),
        };
        let (eq_count, ne_count) = (count(&mut engine, eq), count(&mut engine, ne));
        assert_eq!(eq_count + ne_count, n as f64, "{eq} + {ne}");
        assert_eq!(
            eq_count,
            bits.iter().filter(|b| **b).count() as f64,
            "COUNTIF {eq}"
        );
        text_only.push(bits);
    }

    // A number below the text sends the column to the per-cell mask.
    engine
        .set_cell_value("S", n as u32 + 1, 1, LiteralValue::Number(7.0))
        .unwrap();
    for ((eq, ne), bits) in pairs.into_iter().zip(text_only) {
        let eq_pred = crate::args::parse_criteria(&LiteralValue::Text(eq.into())).unwrap();
        let ne_pred = crate::args::parse_criteria(&LiteralValue::Text(ne.into())).unwrap();
        let view = engine.resolve_range_view(&mixed_range, "S").unwrap();
        let eq_mask = engine.build_criteria_mask(&view, 0, &eq_pred).unwrap();
        let ne_mask = engine.build_criteria_mask(&view, 0, &ne_pred).unwrap();
        for (row, cell) in cells.iter().enumerate() {
            assert_eq!(mask_bit(&eq_mask, row), bits[row], "mixed {eq} on {cell}");
            assert_eq!(mask_bit(&ne_mask, row), !bits[row], "mixed {ne} on {cell}");
        }
    }
}
