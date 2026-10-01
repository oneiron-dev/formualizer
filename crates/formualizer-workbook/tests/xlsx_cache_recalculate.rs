#![cfg(feature = "xlsx-recalc")]
use calamine::{Data, Reader, Xlsx};
use formualizer_workbook::{XlsxRecalculateOptions, recalculate_xlsx_bytes};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};
use zip::{ZipArchive, ZipWriter};

const SHEET: &str = "xl/worksheets/sheet1.xml";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
fn parts(rows: &str) -> BTreeMap<String, String> {
    [
        ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"bin\" ContentType=\"application/octet-stream\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>".to_owned()),
        ("_rels/.rels",format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        ("xl/workbook.xml",format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{OFFICE}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        ("xl/_rels/workbook.xml.rels",format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>")),
        (SHEET,format!("<worksheet xmlns=\"{MAIN}\"><sheetData>{rows}</sheetData></worksheet>")),
        ("custom/opaque.bin","do not touch".to_owned()),
    ].into_iter().map(|(k,v)|(k.to_owned(),v)).collect()
}
fn pack(parts: &BTreeMap<String, impl AsRef<[u8]>>) -> Vec<u8> {
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    z.set_comment("archive-comment");
    let options = zip::write::SimpleFileOptions::default()
        .unix_permissions(0o640)
        .last_modified_time(zip::DateTime::from_date_and_time(2020, 1, 2, 3, 4, 6).unwrap());
    for (name, body) in parts {
        z.start_file(name, options).unwrap();
        z.write_all(body.as_ref()).unwrap();
    }
    z.finish().unwrap().into_inner()
}
fn single(formula: &str, cache: &str) -> BTreeMap<String, String> {
    parts(&format!(
        "<row r=\"1\"><c r=\"A1\"><f>{formula}</f>{cache}</c></row>"
    ))
}
fn fixture(formula: &str, cache: &str) -> Vec<u8> {
    pack(&single(formula, &format!("<v>{cache}</v>")))
}
fn member(bytes: &[u8], name: &str) -> String {
    String::from_utf8(member_bytes(bytes, name)).unwrap()
}
fn member_bytes(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut z = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut b = Vec::new();
    z.by_name(name).unwrap().read_to_end(&mut b).unwrap();
    b
}
fn data(bytes: &[u8], row: u32) -> Data {
    let mut x = Xlsx::new(Cursor::new(bytes)).unwrap();
    x.worksheet_range("Sheet1")
        .unwrap()
        .get_value((row, 0))
        .cloned()
        .unwrap_or(Data::Empty)
}
fn reject(parts: &BTreeMap<String, String>) {
    assert!(recalculate_xlsx_bytes(&pack(parts), XlsxRecalculateOptions::default()).is_err());
}
#[test]
fn stale_cache_and_untouched_members_and_metadata() {
    let input = fixture("1+1", "99");
    let out = recalculate_xlsx_bytes(&input, XlsxRecalculateOptions::default()).unwrap();
    assert_eq!(
        (
            out.formula_cells,
            out.cache_cells_changed,
            out.worksheet_parts_changed
        ),
        (1, 1, 1)
    );
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
    let mut before = ZipArchive::new(Cursor::new(&input)).unwrap();
    let mut after = ZipArchive::new(Cursor::new(&out.bytes)).unwrap();
    assert_eq!(before.comment(), after.comment());
    assert_eq!(before.len(), after.len());
    for i in 0..before.len() {
        let a = before.by_index(i).unwrap();
        let b = after.by_index(i).unwrap();
        assert_eq!(a.name(), b.name());
        assert_eq!(a.last_modified(), b.last_modified());
        assert_eq!(a.unix_mode(), b.unix_mode());
        assert_eq!(a.compression(), b.compression());
        if a.name() != SHEET {
            assert_eq!(
                &input[a.data_start() as usize..(a.data_start() + a.compressed_size()) as usize],
                &out.bytes
                    [b.data_start() as usize..(b.data_start() + b.compressed_size()) as usize]
            );
        }
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, XlsxRecalculateOptions::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn exact_noops() {
    for input in [
        fixture("1+1", "2"),
        pack(&parts("<row r=\"1\"><c r=\"A1\"><v>2</v></c></row>")),
        pack(&parts("")),
    ] {
        let out = recalculate_xlsx_bytes(&input, XlsxRecalculateOptions::default()).unwrap();
        assert_eq!(out.bytes, input);
        assert_eq!(out.cache_cells_changed, 0);
    }
}
#[test]
fn missing_cache_and_typed_cache_repairs() {
    for (formula, expected, fragment) in [
        ("1+1", Data::Float(2.0), "<v>2</v>"),
        ("TRUE()", Data::Bool(true), "t=\"b\""),
        (
            "&quot;hi &amp; &lt; 💡&quot;",
            Data::String("hi & < 💡".into()),
            "<v>hi &amp; &lt; 💡</v>",
        ),
        ("&quot;&quot;", Data::String(String::new()), "<v></v>"),
        ("1/0", Data::Error(calamine::CellErrorType::Div0), "t=\"e\""),
    ] {
        let input = pack(&single(formula, ""));
        let out = recalculate_xlsx_bytes(&input, XlsxRecalculateOptions::default()).unwrap();
        assert_eq!(data(&out.bytes, 0), expected, "{formula}");
        assert!(member(&out.bytes, SHEET).contains(fragment));
        assert_eq!(
            recalculate_xlsx_bytes(&out.bytes, XlsxRecalculateOptions::default())
                .unwrap()
                .bytes,
            out.bytes
        );
    }
}
#[test]
fn source_epoch_is_authoritative() {
    for (date1904, expected) in [("0", 1463.0), ("1", 1.0), ("true", 1.0)] {
        let mut p = single("DATE(1904,1,2)", "<v>99</v>");
        let wb = p.get_mut("xl/workbook.xml").unwrap();
        *wb = wb.replace(
            "<sheets>",
            &format!("<workbookPr date1904=\"{date1904}\"/><sheets>"),
        );
        let out = recalculate_xlsx_bytes(&pack(&p), XlsxRecalculateOptions::default()).unwrap();
        assert_eq!(data(&out.bytes, 0), Data::Float(expected));
        assert_eq!(member(&out.bytes, "xl/workbook.xml"), p["xl/workbook.xml"]);
    }
}
#[test]
fn shared_formula_text_is_untouched() {
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><f t=\"shared\" si=\"0\" ref=\"A1:A3\">ROW()</f><v>99</v></c></row><row r=\"2\"><c r=\"A2\"><f t=\"shared\" si=\"0\"/><v>99</v></c></row><row r=\"3\"><c r=\"A3\"><f t=\"shared\" si=\"0\"/><v>99</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), XlsxRecalculateOptions::default()).unwrap();
    assert_eq!(out.formula_cells, 3);
    for r in 0..3 {
        assert_eq!(data(&out.bytes, r), Data::Float(f64::from(r + 1)));
    }
    let xml = member(&out.bytes, SHEET);
    assert!(xml.contains("<f t=\"shared\" si=\"0\" ref=\"A1:A3\">ROW()</f>"));
    assert_eq!(xml.matches("<f t=\"shared\" si=\"0\"/>").count(), 2);
}
#[test]
fn prefixes_quotes_and_unknown_children_survive() {
    let mut p = single("1+1", "<v>99</v>");
    p.insert(SHEET.into(),format!("<x:worksheet xmlns:x='{MAIN}' xmlns:u='urn:opaque'><x:sheetData><x:row r='1'><x:c r='A1' t='str' u:note='a&amp;&#13;'><x:f>1+1</x:f><x:v>old</x:v></x:c></x:row></x:sheetData><u:payload token='keep'/></x:worksheet>"));
    let out = recalculate_xlsx_bytes(&pack(&p), XlsxRecalculateOptions::default()).unwrap();
    assert_eq!(
        member(&out.bytes, SHEET),
        p[SHEET]
            .replace("t='str'", "")
            .replace(">old</x:v>", ">2</x:v>")
    );
}
#[test]
fn malformed_and_unsupported_worksheet_matrix() {
    let original = single("1+1", "<v>99</v>");
    for xml in [
        original[SHEET].replace("<f>", "<f t=\"array\">"),
        original[SHEET].replace("<f>", "<f t=\"dataTable\">"),
        original[SHEET].replace("<f>", "<f t=\"shared\" si=\"8\">"),
        original[SHEET].replace("r=\"A1\"", "r=\"A0\""),
        original[SHEET].replace("r=\"A1\"", "r=\"XFE1\""),
        original[SHEET].replace("r=\"A1\"", "r=\"A2\""),
        original[SHEET].replace("r=\"A1\"", "r=\"$A$1\""),
        original[SHEET].replace("r=\"A1\"", "r=\"A1\" cm=\"1\""),
        original[SHEET].replace("<f>", "<f xmlns=\"urn:foreign\">"),
        original[SHEET].replace("<v>99</v>", "<v>99</v><v>1</v>"),
        original[SHEET].replace("<v>99</v>", "<v>&unknown;</v>"),
        original[SHEET].replace("<v>99</v>", "<v>&#0;</v>"),
        original[SHEET].replace("</row>", "</c>"),
        format!("<!DOCTYPE worksheet [<!ENTITY e 'x'>]>{}", original[SHEET]),
        original[SHEET].replace(
            "<sheetData>",
            "<dimension ref=\"A1:XFD1048576\"/><sheetData>",
        ),
        original[SHEET].replace(
            "<sheetData>",
            "<dimension ref=\"A1\"/><dimension ref=\"A1\"/><sheetData>",
        ),
        original[SHEET].replace("<sheetData>", "<dimension ref=\"B1\"/><sheetData>"),
    ] {
        let mut p = original.clone();
        p.insert(SHEET.into(), xml);
        reject(&p);
    }
}
#[test]
fn package_mapping_and_signature_rejections() {
    let original = single("1+1", "<v>99</v>");
    for (name, old, new) in [
        (
            "_rels/.rels",
            "Target=\"xl/workbook.xml\"",
            "Target=\"elsewhere.xml\"",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "Target=\"worksheets/sheet1.xml\"",
            "Target=\"../../../escape.xml\"",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "/worksheet\"",
            "/chartsheet\"",
        ),
        ("xl/workbook.xml", MAIN, "urn:foreign"),
        (
            "[Content_Types].xml",
            "spreadsheetml.worksheet+xml",
            "spreadsheetml.styles+xml",
        ),
    ] {
        let mut p = original.clone();
        let s = p.get_mut(name).unwrap();
        *s = s.replace(old, new);
        reject(&p);
    }
    let mut p = original;
    p.insert("_xmlsignatures/sig1.xml".into(), "<Signature/>".into());
    reject(&p);
}
const WORKBOOK_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const VBA_PROJECT: &str = "xl/vbaProject.bin";
/// Bytes for the VBA project part: the compound-file (CFB) signature padded
/// with zeros to one 512-byte header sector. Not a readable VBA project; the
/// recalculation treats the part as opaque bytes.
fn vba_project() -> Vec<u8> {
    let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    bytes.resize(512, 0);
    bytes
}
/// `single(formula, cache)` saved as Excel saves it with the workbook part
/// typed `content_type`; a macro-enabled kind is also wired for a VBA project
/// the way Excel writes one (Default `bin` type, workbook `vbaProject`
/// relationship, code names on the workbook and sheet). `pack_kind` adds the
/// project part itself.
fn workbook_kind(content_type: &str, formula: &str, cache: &str) -> BTreeMap<String, String> {
    let mut p = single(formula, cache);
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace(WORKBOOK_TYPE, content_type);
    if content_type.contains("macroEnabled") {
        *ct = ct.replace(
            "ContentType=\"application/octet-stream\"",
            "ContentType=\"application/vnd.ms-office.vbaProject\"",
        );
        let rels = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
        *rels = rels.replace("</Relationships>", "<Relationship Id=\"rId9\" Type=\"http://schemas.microsoft.com/office/2006/relationships/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>");
        let workbook = p.get_mut("xl/workbook.xml").unwrap();
        *workbook = workbook.replace(
            "<sheets>",
            "<workbookPr codeName=\"ThisWorkbook\"/><sheets>",
        );
        let sheet = p.get_mut(SHEET).unwrap();
        *sheet = sheet.replace("<sheetData>", "<sheetPr codeName=\"Sheet1\"/><sheetData>");
    }
    p
}
/// `p` packed with the VBA project part when its workbook refers to one.
fn pack_kind(p: &BTreeMap<String, String>) -> Vec<u8> {
    let mut parts: BTreeMap<String, Vec<u8>> = p
        .iter()
        .map(|(name, body)| (name.clone(), body.clone().into_bytes()))
        .collect();
    if p["xl/_rels/workbook.xml.rels"].contains("relationships/vbaProject") {
        parts.insert(VBA_PROJECT.into(), vba_project());
    }
    pack(&parts)
}
#[test]
fn macro_enabled_and_template_workbooks_recalculate_like_xlsx() {
    for content_type in [
        "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
        "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        "application/vnd.ms-excel.template.macroEnabled.main+xml",
        "application/vnd.ms-excel.addin.macroEnabled.main+xml",
    ] {
        let p = workbook_kind(content_type, "1+1", "<v>99</v>");
        let input = pack_kind(&p);
        let out = recalculate_xlsx_bytes(&input, XlsxRecalculateOptions::default())
            .unwrap_or_else(|e| panic!("{content_type}: {e:?}"));
        assert_eq!(
            (out.cache_cells_changed, out.worksheet_parts_changed),
            (1, 1)
        );
        assert_eq!(data(&out.bytes, 0), Data::Float(2.0), "{content_type}");
        // Every part except the worksheet is copied byte for byte: the VBA
        // project, its relationship and the content types too.
        let mut before = ZipArchive::new(Cursor::new(&input)).unwrap();
        let mut after = ZipArchive::new(Cursor::new(&out.bytes)).unwrap();
        assert_eq!(before.len(), after.len());
        for i in 0..before.len() {
            let a = before.by_index(i).unwrap();
            let b = after.by_index(i).unwrap();
            assert_eq!(a.name(), b.name());
            if a.name() != SHEET {
                assert_eq!(
                    &input
                        [a.data_start() as usize..(a.data_start() + a.compressed_size()) as usize],
                    &out.bytes
                        [b.data_start() as usize..(b.data_start() + b.compressed_size()) as usize],
                    "{content_type}: {}",
                    a.name()
                );
            }
        }
        let types = member(&out.bytes, "[Content_Types].xml");
        assert!(types.contains(&format!("ContentType=\"{content_type}\"")));
        if content_type.contains("macroEnabled") {
            // The VBA project is opaque bytes to the recalculation: a CFB
            // header that is not a readable project, written back unchanged.
            assert_eq!(member_bytes(&input, VBA_PROJECT), vba_project());
            assert_eq!(member_bytes(&out.bytes, VBA_PROJECT), vba_project());
            assert!(types.contains("application/vnd.ms-office.vbaProject"));
            assert!(
                member(&out.bytes, "xl/_rels/workbook.xml.rels")
                    .contains("relationships/vbaProject\" Target=\"vbaProject.bin\"")
            );
        }
        // A current macro-enabled package is an exact no-op.
        let again = recalculate_xlsx_bytes(&out.bytes, XlsxRecalculateOptions::default()).unwrap();
        assert_eq!(again.bytes, out.bytes);
    }
}
#[test]
fn workbook_part_content_type_must_be_a_workbook_type() {
    let xlsm = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
    for (part, content_type) in [
        // The binary (.xlsb) workbook, a word-processing document and a
        // worksheet type are not SpreadsheetML workbook parts.
        (
            "/xl/workbook.xml",
            "application/vnd.ms-excel.sheet.binary.macroEnabled.main",
        ),
        (
            "/xl/workbook.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        ),
        (
            "/xl/workbook.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
        ),
        (
            "/xl/workbook.xml",
            "application/vnd.ms-excel.sheet.macroEnabled+xml",
        ),
        // A workbook type does not type a worksheet.
        ("/xl/worksheets/sheet1.xml", xlsm),
        // Without its override the workbook part is plain application/xml.
        ("/xl/workbook.xml", ""),
    ] {
        let mut p = workbook_kind(xlsm, "1+1", "<v>99</v>");
        let ct = p.get_mut("[Content_Types].xml").unwrap();
        if content_type.is_empty() {
            *ct = ct.replace(
                &format!("<Override PartName=\"{part}\" ContentType=\"{xlsm}\"/>"),
                "",
            );
        } else {
            let start = ct.find(&format!("PartName=\"{part}\"")).unwrap();
            let open = start + ct[start..].find("ContentType=\"").unwrap() + 13;
            let close = open + ct[open..].find('"').unwrap();
            ct.replace_range(open..close, content_type);
        }
        let error = recalculate_xlsx_bytes(&pack_kind(&p), XlsxRecalculateOptions::default())
            .expect_err(content_type);
        assert!(
            format!("{error:?}").contains("part/content-type mismatch"),
            "{content_type}: {error:?}"
        );
    }
}
#[test]
fn cell_filename_extension_follows_the_workbook_type() {
    let filename = "CELL(\"filename\",A1)";
    for (content_type, extension) in [
        (WORKBOOK_TYPE, "xlsx"),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            "xltx",
        ),
        (
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            "xlsm",
        ),
        (
            "application/vnd.ms-excel.template.macroEnabled.main+xml",
            "xltm",
        ),
        (
            "application/vnd.ms-excel.addin.macroEnabled.main+xml",
            "xlam",
        ),
    ] {
        let p = workbook_kind(content_type, filename, "<v>0</v>");
        let out = recalculate_xlsx_bytes(&pack_kind(&p), Default::default()).unwrap();
        let sheet = member(&out.bytes, SHEET);
        assert!(
            sheet.contains(&format!("<v>[workbook.{extension}]Sheet1</v>")),
            "{content_type}: {sheet}"
        );
    }
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cell_filename_names_the_input_file() {
    use formualizer_workbook::recalculate_xlsx_file;
    let filename = "CELL(\"filename\",A1)";
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.xlsx");
    for (content_type, file) in [
        (
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            "Q3 Budget.xlsm",
        ),
        (WORKBOOK_TYPE, "Q3 Budget.xlsx"),
    ] {
        let input = dir.path().join(file);
        std::fs::write(
            &input,
            pack_kind(&workbook_kind(content_type, filename, "<v>0</v>")),
        )
        .unwrap();
        recalculate_xlsx_file(&input, Some(&output), Default::default()).unwrap();
        let sheet = member(&std::fs::read(&output).unwrap(), SHEET);
        assert!(sheet.contains(&format!("<v>[{file}]Sheet1</v>")), "{sheet}");
        // A name the host gives wins over the file's.
        let mut options = XlsxRecalculateOptions::default();
        options.eval_config.workbook_file_name = Some(format!("Host {file}"));
        recalculate_xlsx_file(&input, None, options).unwrap();
        let sheet = member(&std::fs::read(&input).unwrap(), SHEET);
        assert!(
            sheet.contains(&format!("<v>[Host {file}]Sheet1</v>")),
            "{sheet}"
        );
    }
}
#[test]
fn limits_and_precancellation() {
    let input = fixture("1+1", "99");
    for which in 0..7 {
        let mut o = XlsxRecalculateOptions::default();
        match which {
            0 => o.limits.max_input_bytes = 1,
            1 => o.limits.max_entries = 1,
            2 => o.limits.max_expanded_bytes = 1,
            3 => o.limits.max_worksheet_bytes = 1,
            4 => o.limits.max_formula_cells = 0,
            5 => o.limits.max_xml_depth = 1,
            _ => o.limits.max_cells = 0,
        };
        assert!(recalculate_xlsx_bytes(&input, o).is_err(), "limit {which}");
    }
    let cancel = formualizer_eval::engine::CancelToken::new();
    cancel.cancel();
    let o = XlsxRecalculateOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    assert!(recalculate_xlsx_bytes(&input, o).is_err());
}
#[test]
fn unsupported_spill_does_not_return_a_partial_package() {
    // A dynamic array that now spills past the extent recorded in the file.
    let p = parts(
        "<row r=\"1\"><c r=\"A1\" cm=\"1\"><f t=\"array\" ref=\"A1\">SEQUENCE(2)</f><v>99</v></c></row>",
    );
    assert!(recalculate_xlsx_bytes(&pack(&p), XlsxRecalculateOptions::default()).is_err());
    // Without the array flag the formula is a legacy one: its top-left value.
    let out = recalculate_xlsx_bytes(&fixture("SEQUENCE(2)", "99"), Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(1.0));
}
#[test]
fn error_locations_are_bounded() {
    let o = XlsxRecalculateOptions {
        error_location_limit: 0,
        ..Default::default()
    };
    let out = recalculate_xlsx_bytes(&fixture("1/0", "99"), o).unwrap();
    assert_eq!(out.summary.errors, 1);
    let error = &out.summary.error_summary["#DIV/0!"];
    assert_eq!(error.locations.len(), 0);
    assert_eq!(error.locations_truncated, 1);
}
#[test]
fn typed_text_controls_fail_instead_of_silent_corruption() {
    for formula in ["CHAR(1)", "&quot;_x0041_&quot;"] {
        assert!(
            recalculate_xlsx_bytes(&fixture(formula, "99"), XlsxRecalculateOptions::default())
                .is_err()
        );
    }
}
#[test]
fn modern_scalar_errors_are_cached_as_value_errors() {
    // Excel caches #SPILL! and #CALC! as #VALUE!; the summary keeps the kind.
    // New functions are spelled as Excel saves them (_xlfn.).
    for (formula, token) in [
        (
            " cm=\"1\"><f t=\"array\" ref=\"A1\">_xlfn.SEQUENCE(2)</f>",
            "#SPILL!",
        ),
        ("><f>_xlfn._xlws.FILTER(A2:A2,FALSE)</f>", "#CALC!"),
    ] {
        let p = parts(&format!(
            "<row r=\"1\"><c r=\"A1\"{formula}<v>99</v></c></row><row r=\"2\"><c r=\"A2\"><v>7</v></c></row>"
        ));
        let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
        assert_eq!(out.summary.errors, 1);
        assert_eq!(out.summary.error_summary[token].count, 1);
        assert!(member(&out.bytes, SHEET).contains("t=\"e\""));
        assert!(member(&out.bytes, SHEET).contains("</f><v>#VALUE!</v>"));
        assert_eq!(
            data(&out.bytes, 0),
            Data::Error(calamine::CellErrorType::Value)
        );
        let again = recalculate_xlsx_bytes(&out.bytes, Default::default()).unwrap();
        assert_eq!(again.bytes, out.bytes);
        assert_eq!(again.summary.errors, 1);
    }
    // As Excel saved it: a #SPILL! cached as #VALUE! and tagged #SPILL!.
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"e\" cm=\"1\" vm=\"1\"><f t=\"array\" ref=\"A1\">_xlfn.SEQUENCE(2)</f><v>#VALUE!</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>7</v></c></row>";
    let input = pack(&with_rich_errors(parts(rows), &[8]));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!((out.summary.errors, out.cache_cells_changed), (1, 0));
    assert_eq!(out.bytes, input);
    // A genuine #VALUE! is unchanged too; other errors keep their codes.
    for (formula, token) in [
        ("&quot;a&quot;+1", "#VALUE!"),
        ("1/0", "#DIV/0!"),
        ("NA()", "#N/A"),
    ] {
        let out = recalculate_xlsx_bytes(&fixture(formula, "99"), Default::default()).unwrap();
        assert!(
            member(&out.bytes, SHEET).contains(&format!("t=\"e\"><f>{formula}</f><v>{token}</v>"))
        );
    }
}
#[test]
fn defined_names_are_evaluated_without_metadata_rewrite() {
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><f>Answer+1</f><v>99</v></c></row><row r=\"2\"><c r=\"A2\"><v>7</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb=wb.replace("</workbook>","<definedNames><definedName name=\"Answer\">Sheet1!$A$2</definedName></definedNames></workbook>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(8.0));
    assert_eq!(member(&out.bytes, "xl/workbook.xml"), p["xl/workbook.xml"]);
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</definedNames>",
        "<definedName name=\"answer\">Sheet1!$A$1</definedName></definedNames>",
    );
    reject(&p);
}
#[test]
fn nonportable_literal_errors_are_explicitly_rejected() {
    let p = parts(
        "<row r=\"1\"><c r=\"A1\" t=\"e\"><v>#SPILL!</v></c><c r=\"B1\"><f>IFERROR(A1,0)</f><v>99</v></c></row>",
    );
    let error = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap_err();
    assert!(
        matches!(error,formualizer_workbook::IoError::Unsupported{feature,..} if feature.contains("literal error"))
    );
    // Table parts are supported (see worksheet_tables_answer_structured_references);
    // an empty tableParts list is just metadata.
    let mut p = single("1+1", "<v>99</v>");
    let s = p.get_mut(SHEET).unwrap();
    *s = s.replace("</worksheet>", "<tableParts count=\"0\"/></worksheet>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
}
#[test]
fn engine_specific_errors_are_unsupported_results_not_invented_excel_tokens() {
    // R1C1-style INDIRECT still evaluates to the engine-only #N/IMPL!.
    let error = recalculate_xlsx_bytes(
        &fixture("INDIRECT(&quot;R2C2:R3C3&quot;,FALSE)", "99"),
        Default::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&error,formualizer_workbook::IoError::Unsupported{feature,context}
            if feature=="formula result is not current"
                || (feature.contains("no approved XLSX cache encoding") && context=="#N/IMPL!")),
        "unexpected error: {error:?}"
    );
}
#[test]
fn arbitrary_stale_error_cache_is_not_evaluator_authority() {
    let mut p = single("1+1", "<v>#FUTURE_ERROR!</v>");
    let s = p.get_mut(SHEET).unwrap();
    *s = s.replace("r=\"A1\"", "r=\"A1\" t=\"e\"");
    let output = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    assert_eq!(data(&output.bytes, 0), Data::Float(2.0));
}
#[test]
fn cached_text_empty_element_is_an_exact_noop() {
    let mut p = single("&quot;&quot;", "<v/>");
    let xml = p.get_mut(SHEET).unwrap();
    *xml = xml.replace("r=\"A1\"", "r=\"A1\" t=\"str\"");
    let input = pack(&p);
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.bytes, input);
    assert_eq!(data(&out.bytes, 0), Data::String(String::new()));
}
#[test]
fn one_cell_dynamic_result_does_not_require_geometry_writeback() {
    let out = recalculate_xlsx_bytes(&fixture("SEQUENCE(1)", "99"), Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(1.0));
}
#[test]
fn scalar_ingestion_cannot_silently_drop_xml_text() {
    for payload in ["1&#50;", "1<!--split-->2", "<![CDATA[12]]>"] {
        let p = parts(&format!(
            "<row r=\"1\"><c r=\"A1\"><v>{payload}</v></c><c r=\"B1\"><f>A1+1</f><v>99</v></c></row>"
        ));
        reject(&p);
    }
    let input = pack(&single("12", "<v>1&#50;</v>"));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.bytes, input);
    let p = single("1<![CDATA[+1]]>", "<v>99</v>");
    reject(&p);
}
#[test]
fn serial_egress_preserves_phantom_day_and_fractional_dates() {
    for value in ["60", "60.125", "-0.125"] {
        let mut p = single(value, "<v>99</v>");
        let worksheet = p.get_mut(SHEET).unwrap();
        *worksheet = worksheet.replace("r=\"A1\"", "r=\"A1\" s=\"0\"");
        p.insert("xl/styles.xml".into(),format!("<styleSheet xmlns=\"{MAIN}\"><cellXfs count=\"1\"><xf numFmtId=\"14\"/></cellXfs></styleSheet>"));
        let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
        *rel=rel.replace("</Relationships>",&format!("<Relationship Id=\"style\" Type=\"{OFFICE}/styles\" Target=\"styles.xml\"/></Relationships>"));
        let ct = p.get_mut("[Content_Types].xml").unwrap();
        *ct=ct.replace("</Types>","<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>");
        let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
        assert!(member(&out.bytes, SHEET).contains(&format!("<v>{value}</v>")));
        assert_eq!(member(&out.bytes, "xl/styles.xml"), p["xl/styles.xml"]);
    }
}
#[test]
fn numbers_used_as_text_keep_15_significant_digits() {
    // A date-time serial in a date-formatted cell reads as its number, and
    // text formulas see Excel's 15-digit text of it, not the display format.
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>44468.756944444445</v></c>\
         <c r=\"B1\" t=\"str\"><f>RIGHT(A1,8)</f><v>x</v></c>\
         <c r=\"C1\" t=\"str\"><f>A1&amp;\"\"</f><v>x</v></c>\
         <c r=\"D1\" t=\"str\"><f>CONCAT(1/3,\"|\",2^53)</f><v>x</v></c>\
         <c r=\"E1\"><f>LEN(A1)</f><v>0</v></c></row>",
    );
    p.insert("xl/styles.xml".into(),format!("<styleSheet xmlns=\"{MAIN}\"><cellXfs count=\"2\"><xf numFmtId=\"0\"/><xf numFmtId=\"22\" applyNumberFormat=\"1\"/></cellXfs></styleSheet>"));
    let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rel=rel.replace("</Relationships>",&format!("<Relationship Id=\"style\" Type=\"{OFFICE}/styles\" Target=\"styles.xml\"/></Relationships>"));
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct=ct.replace("</Types>","<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let mut x = Xlsx::new(Cursor::new(out.bytes)).unwrap();
    let range = x.worksheet_range("Sheet1").unwrap();
    let cell = |col: u32| range.get_value((0, col)).cloned().unwrap_or(Data::Empty);
    assert_eq!(cell(1), Data::String("69444444".into()));
    assert_eq!(cell(2), Data::String("44468.7569444444".into()));
    assert_eq!(
        cell(3),
        Data::String("0.333333333333333|9007199254740990".into())
    );
    assert_eq!(cell(4), Data::Float(16.0));
}
fn h16(b: &[u8], i: usize) -> usize {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap()) as usize
}
fn h32(b: &[u8], i: usize) -> usize {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize
}
fn directory(bytes: &[u8]) -> (Vec<usize>, usize) {
    let archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut at = archive.central_directory_start() as usize;
    let mut result = Vec::new();
    for _ in 0..archive.len() {
        result.push(at);
        at += 46 + h16(bytes, at + 28) + h16(bytes, at + 30) + h16(bytes, at + 32);
    }
    (result, at)
}
#[test]
fn zip_metadata_is_not_normalized_even_for_changed_members() {
    let mut input = fixture("1+1", "99");
    let (headers, _) = directory(&input);
    for &at in &headers {
        input[at + 4] = 20;
        input[at + 36] = 1;
        input[at + 38] |= 0x20;
    }
    let output = recalculate_xlsx_bytes(&input, Default::default())
        .unwrap()
        .bytes;
    let (after, _) = directory(&output);
    for (&a, &b) in headers.iter().zip(&after) {
        let length = 46 + h16(&input, a + 28) + h16(&input, a + 30) + h16(&input, a + 32);
        for i in 0..length {
            if !(16..28).contains(&i) && !(42..46).contains(&i) {
                assert_eq!(input[a + i], output[b + i], "central field {i}");
            }
        }
        let la = h32(&input, a + 42);
        let lb = h32(&output, b + 42);
        let local_length = 30 + h16(&input, la + 26) + h16(&input, la + 28);
        for i in 0..local_length {
            if !(14..26).contains(&i) {
                assert_eq!(input[la + i], output[lb + i], "local field {i}");
            }
        }
    }
}
/// Splice one local-header extra record into every member of a packed archive
/// (ZIP7 refuses to author unreserved IDs), relocating the central directory.
fn pack_with_local_extra(parts: &BTreeMap<String, String>, id: u16, body: &[u8]) -> Vec<u8> {
    let input = pack(parts);
    let (headers, footer) = directory(&input);
    let central = h32(&input, footer + 16);
    let mut record = id.to_le_bytes().to_vec();
    record.extend_from_slice(&(body.len() as u16).to_le_bytes());
    record.extend_from_slice(body);
    let mut locals: Vec<usize> = headers.iter().map(|&a| h32(&input, a + 42)).collect();
    locals.sort_unstable();
    let mut out = Vec::new();
    let mut moved = BTreeMap::new();
    for (i, &local) in locals.iter().enumerate() {
        let header = out.len();
        moved.insert(local, header);
        let fixed = local + 30 + h16(&input, local + 26);
        assert_eq!(h16(&input, local + 28), 0);
        out.extend_from_slice(&input[local..fixed]);
        out[header + 28..header + 30].copy_from_slice(&(record.len() as u16).to_le_bytes());
        out.extend_from_slice(&record);
        let next = locals.get(i + 1).copied().unwrap_or(central);
        out.extend_from_slice(&input[fixed..next]);
    }
    let new_central = out.len();
    out.extend_from_slice(&input[central..]);
    for &a in &headers {
        let at = new_central + a - central;
        let local = moved[&h32(&input, a + 42)] as u32;
        out[at + 42..at + 46].copy_from_slice(&local.to_le_bytes());
    }
    let at = new_central + footer - central;
    out[at + 16..at + 20].copy_from_slice(&(new_central as u32).to_le_bytes());
    out
}
#[test]
fn office_growth_hint_padding_is_admitted_and_retained() {
    // Excel pads every local header with a 0xA220 growth hint (signature
    // 0xA028, padding length, zero padding); it describes no sizes/offsets.
    let mut hint = vec![0x28, 0xA0, 0xFC, 0x00];
    hint.resize(256, 0);
    let input = pack_with_local_extra(&single("1+1", "<v>99</v>"), 0xA220, &hint);
    let (headers, _) = directory(&input);
    let local = h32(&input, headers[0] + 42);
    assert_eq!(h16(&input, local + 28), 260);
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.cache_cells_changed, 1);
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
    let (after, _) = directory(&out.bytes);
    for (&a, &b) in headers.iter().zip(&after) {
        let la = h32(&input, a + 42);
        let lb = h32(&out.bytes, b + 42);
        let extra = 30 + h16(&input, la + 26);
        assert_eq!(
            input[la + extra..la + extra + 260],
            out.bytes[lb + extra..lb + extra + 260]
        );
    }
    assert_eq!(member(&out.bytes, "custom/opaque.bin"), "do not touch");
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
    // Other local extras (here an extended timestamp) stay unsupported.
    let other = pack_with_local_extra(&single("1+1", "<v>99</v>"), 0x5455, &[1, 0, 0, 0, 0]);
    assert!(recalculate_xlsx_bytes(&other, Default::default()).is_err());
}
#[test]
fn local_headers_differing_in_deflate_hints_and_time_are_admitted() {
    // Excel writes some local headers with deflate speed hints (flag bits 1-2)
    // and a zero timestamp that the central directory does not repeat.
    let input = pack(&single("1+1", "<v>99</v>"));
    let (headers, _) = directory(&input);
    let mut hinted = input.clone();
    for &a in &headers {
        let local = h32(&input, a + 42);
        let flags = h16(&input, local + 6) as u16 | 0b110;
        hinted[local + 6..local + 8].copy_from_slice(&flags.to_le_bytes());
        hinted[local + 10..local + 14].copy_from_slice(&[0, 0, 33, 0]);
    }
    let out = recalculate_xlsx_bytes(&hinted, Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
    // Any other flag difference still refuses the package.
    let mut other = input.clone();
    let local = h32(&input, headers[0] + 42);
    let flags = h16(&input, local + 6) as u16 | 0x0800;
    other[local + 6..local + 8].copy_from_slice(&flags.to_le_bytes());
    assert!(recalculate_xlsx_bytes(&other, Default::default()).is_err());
}
fn with_metadata(mut p: BTreeMap<String, String>, metadata: &str) -> BTreeMap<String, String> {
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>","<Override PartName=\"/xl/metadata.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml\"/></Types>");
    let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rel = rel.replace("</Relationships>",&format!("<Relationship Id=\"rId9\" Type=\"{OFFICE}/sheetMetadata\" Target=\"metadata.xml\"/></Relationships>"));
    p.insert("xl/metadata.xml".into(), format!("<metadata xmlns=\"{MAIN}\" xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\">{metadata}</metadata>"));
    p
}
const XLDAPR: &str = "<metadataTypes count=\"1\"><metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/></metadataTypes><futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}\"><xda:dynamicArrayProperties fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
#[test]
fn single_cell_array_formulas_evaluate_with_array_semantics() {
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f t=\"array\" ref=\"B1\">SUM(A1:A3*A1:A3)</f><v>0</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\" cm=\"1\"><f t=\"array\" ref=\"B2\">SUM((A1:A3&gt;1)*A1:A3)</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c></row>";
    // Legacy CSE (no metadata) and a dynamic-array formula flagged by XLDAPR.
    let input = pack(&with_metadata(parts(rows), XLDAPR));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.cache_cells_changed, 2);
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains("ref=\"B1\">SUM(A1:A3*A1:A3)</f><v>14</v>"));
    assert!(sheet.contains("<c r=\"B2\" cm=\"1\"><f t=\"array\" ref=\"B2\">"));
    assert!(sheet.contains("*A1:A3)</f><v>5</v>"), "{sheet}");
    assert_eq!(
        member(&out.bytes, "xl/metadata.xml"),
        member(&input, "xl/metadata.xml")
    );
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn misplaced_arrays_and_rich_value_metadata_stay_unsupported() {
    // A multi-cell array anchored away from its extent's top-left.
    let multi = "<row r=\"2\"><c r=\"B2\"><f t=\"array\" ref=\"A1:B2\">{1;2}</f><v>1</v></c></row>";
    reject(&parts(multi));
    // `cm` outside an array formula is not dynamic-array metadata.
    let flagged = "<row r=\"1\"><c r=\"A1\" cm=\"1\"><f>1+1</f><v>2</v></c></row>";
    reject(&with_metadata(parts(flagged), XLDAPR));
    let array =
        "<row r=\"1\"><c r=\"A1\" cm=\"1\"><f t=\"array\" ref=\"A1\">1+1</f><v>2</v></c></row>";
    let rich = format!(
        "{XLDAPR}<valueMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></valueMetadata>"
    );
    reject(&with_metadata(parts(array), &rich));
    let other = XLDAPR.replace("name=\"XLDAPR\" min", "name=\"XLRICHVALUE\" min");
    reject(&with_metadata(parts(array), &other));
}
const RICH_RELS: &str = "http://schemas.microsoft.com/office/2017/06/relationships";
/// Excel's tags for cached errors without a legacy code: value metadata `i`
/// (cell `vm="i+1"`) is an `_error` rich value with `errorType` codes[i].
fn with_rich_errors(mut p: BTreeMap<String, String>, codes: &[u32]) -> BTreeMap<String, String> {
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>","<Override PartName=\"/xl/metadata.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml\"/><Override PartName=\"/xl/richData/rdrichvalue.xml\" ContentType=\"application/vnd.ms-excel.rdrichvalue+xml\"/><Override PartName=\"/xl/richData/rdrichvaluestructure.xml\" ContentType=\"application/vnd.ms-excel.rdrichvaluestructure+xml\"/></Types>");
    let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rel = rel.replace("</Relationships>",&format!("<Relationship Id=\"rId9\" Type=\"{OFFICE}/sheetMetadata\" Target=\"metadata.xml\"/><Relationship Id=\"rId10\" Type=\"{RICH_RELS}/rdRichValue\" Target=\"richData/rdrichvalue.xml\"/><Relationship Id=\"rId11\" Type=\"{RICH_RELS}/rdRichValueStructure\" Target=\"richData/rdrichvaluestructure.xml\"/></Relationships>"));
    let n = codes.len();
    let blocks: String = (0..n).map(|i| format!("<bk><extLst><ext uri=\"{{3e2802c4-a4d2-4d8b-9148-e3be6c30e623}}\"><xlrd:rvb i=\"{i}\"/></ext></extLst></bk>")).collect();
    let records: String = (0..n)
        .map(|i| format!("<bk><rc t=\"2\" v=\"{i}\"/></bk>"))
        .collect();
    let types = XLDAPR.replace("<metadataTypes count=\"1\">", "<metadataTypes count=\"2\">").replace("</metadataTypes>", "<metadataType name=\"XLRICHVALUE\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\"/></metadataTypes>");
    let types = types.replace("<cellMetadata", &format!("<futureMetadata name=\"XLRICHVALUE\" count=\"{n}\">{blocks}</futureMetadata><cellMetadata"));
    p.insert("xl/metadata.xml".into(), format!("<metadata xmlns=\"{MAIN}\" xmlns:xlrd=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata\" xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\">{types}<valueMetadata count=\"{n}\">{records}</valueMetadata></metadata>"));
    let values: String = codes
        .iter()
        .map(|c| format!("<rv s=\"0\"><v>0</v><v>{c}</v><v>0</v><v>3</v></rv>"))
        .collect();
    p.insert("xl/richData/rdrichvalue.xml".into(), format!("<rvData xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata\" count=\"{n}\">{values}</rvData>"));
    p.insert("xl/richData/rdrichvaluestructure.xml".into(), "<rvStructures xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata\" count=\"1\"><s t=\"_error\"><k n=\"colOffset\" t=\"i\"/><k n=\"errorType\" t=\"i\"/><k n=\"rwOffset\" t=\"i\"/><k n=\"subType\" t=\"i\"/></s></rvStructures>".into());
    p
}
#[test]
fn rich_error_tags_are_kept_only_while_the_cell_holds_that_error() {
    // vm 1 tags #SPILL! (errorType 8), vm 2 #CALC! (13), vm 3 #CONNECT! (9).
    // A1 still spills into A2's value, B1 is now 2, C1 is now a genuine
    // #VALUE!, D1 is still #CALC!, E1 is a #SPILL! tagged #CONNECT!.
    // Functions newer than the 2007 file format are saved with Excel's
    // _xlfn. prefix.
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"e\" cm=\"1\" vm=\"1\"><f t=\"array\" ref=\"A1\">_xlfn.SEQUENCE(2)</f><v>#VALUE!</v></c>\
        <c r=\"B1\" t=\"e\" vm=\"1\"><f>1+1</f><v>#VALUE!</v></c>\
        <c r=\"C1\" t=\"e\" vm=\"1\"><f>&quot;a&quot;+1</f><v>#VALUE!</v></c>\
        <c r=\"D1\" t=\"e\" vm=\"2\"><f>_xlfn._xlws.FILTER(A2:A2,FALSE)</f><v>#VALUE!</v></c>\
        <c r=\"E1\" vm=\"3\" t=\"e\" cm=\"1\"><f t=\"array\" ref=\"E1\">_xlfn.SEQUENCE(2)</f><v>#VALUE!</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>7</v></c><c r=\"E2\"><v>7</v></c></row>";
    let mut p = with_rich_errors(parts(rows), &[8, 13, 9]);
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<c r=\"A1\" t=\"e\" cm=\"1\" vm=\"1\">",
        // The removed type attribute leaves its separating space.
        "<c r=\"B1\" ><f>1+1</f><v>2</v>",
        "<c r=\"C1\" t=\"e\"><f>",
        "<c r=\"D1\" t=\"e\" vm=\"2\">",
        "<c r=\"E1\" t=\"e\" cm=\"1\">",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(out.summary.errors, 4);
    assert!(!sheet.contains("ca=\"1\""), "{sheet}");
    for part in ["xl/metadata.xml", "xl/richData/rdrichvalue.xml"] {
        assert_eq!(member(&out.bytes, part), p[part]);
    }
    let again = recalculate_xlsx_bytes(&out.bytes, Default::default()).unwrap();
    assert_eq!(again.bytes, out.bytes);
    // Excel adds the rich value types part (global key flags) when it saves.
    p.insert("xl/richData/rdRichValueTypes.xml".into(), "<rvTypesInfo xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata2\"><global><keyFlags><key name=\"_Self\"><flag name=\"ExcludeFromFile\" value=\"1\"/></key></keyFlags></global></rvTypesInfo>".into());
    assert_eq!(
        recalculate_xlsx_bytes(&pack(&p), Default::default())
            .unwrap()
            .summary
            .errors,
        4
    );
}
#[test]
fn rich_values_other_than_error_tags_stay_unsupported() {
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"e\" vm=\"1\"><f>1+1</f><v>#VALUE!</v></c></row>";
    assert!(
        recalculate_xlsx_bytes(
            &pack(&with_rich_errors(parts(rows), &[8])),
            Default::default()
        )
        .is_ok()
    );
    // A tag on a value that no formula computes, or outside the records.
    for cell in [
        "<c r=\"A1\" t=\"e\" vm=\"1\"><v>#VALUE!</v></c>",
        "<c r=\"A1\" vm=\"1\"/>",
        "<c r=\"A1\" t=\"e\" vm=\"2\"><f>1+1</f><v>#VALUE!</v></c>",
        "<c r=\"A1\" t=\"e\" vm=\"0\"><f>1+1</f><v>#VALUE!</v></c>",
    ] {
        reject(&with_rich_errors(
            parts(&format!("<row r=\"1\">{cell}</row>")),
            &[8],
        ));
    }
    // A rich value that is not an error, such as a picture in a cell.
    let mut image = with_rich_errors(parts(rows), &[8]);
    let s = image
        .get_mut("xl/richData/rdrichvaluestructure.xml")
        .unwrap();
    *s = s.replace("<s t=\"_error\">", "<s t=\"_localImage\">");
    reject(&image);
    // An error key that refers to another rich value (#BUSY! targetValue).
    let mut reference = with_rich_errors(parts(rows), &[8]);
    let s = reference
        .get_mut("xl/richData/rdrichvaluestructure.xml")
        .unwrap();
    *s = s.replace(
        "<k n=\"subType\" t=\"i\"/>",
        "<k n=\"targetValue\" t=\"r\"/>",
    );
    reject(&reference);
    // Other rich data parts (images, arrays, property bags).
    let mut extra = with_rich_errors(parts(rows), &[8]);
    extra.insert("xl/richData/richValueRel.xml".into(), "<richValueRels xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2022/richvaluerel\"/>".into());
    reject(&extra);
    // A rich value reference outside its future-metadata block.
    let mut outside = with_rich_errors(parts(rows), &[8]);
    let m = outside.get_mut("xl/metadata.xml").unwrap();
    *m = m.replace(
        "<bk><extLst><ext uri=\"{3e2802c4-a4d2-4d8b-9148-e3be6c30e623}\"><xlrd:rvb i=\"0\"/></ext></extLst></bk>",
        "<bk/><extLst><ext uri=\"{3e2802c4-a4d2-4d8b-9148-e3be6c30e623}\"><xlrd:rvb i=\"0\"/></ext></extLst>",
    );
    assert!(m.contains("<bk/><extLst>"));
    reject(&outside);
    // A value record must name rich value metadata.
    let mut untyped = with_rich_errors(parts(rows), &[8]);
    let m = untyped.get_mut("xl/metadata.xml").unwrap();
    *m = m.replace(
        "<valueMetadata count=\"1\"><bk><rc t=\"2\"",
        "<valueMetadata count=\"1\"><bk><rc t=\"1\"",
    );
    reject(&untyped);
}
#[test]
fn documented_rich_error_variants_are_admitted() {
    // vm 1 tags #CALC! (13), vm 2 #FIELD! (12). A1 is still #CALC!, B1 is
    // now 2.
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"e\" vm=\"1\"><f>_xlfn._xlws.FILTER(C1:C1,FALSE)</f><v>#VALUE!</v></c>\
        <c r=\"B1\" t=\"e\" vm=\"2\"><f>1+1</f><v>#VALUE!</v></c></row>";
    const STRUCTURES: &str = "xl/richData/rdrichvaluestructure.xml";
    const VALUES: &str = "xl/richData/rdrichvalue.xml";
    let mut p = with_rich_errors(parts(rows), &[13, 12]);
    // Key names are case-insensitive; a #FIELD! names the missing field in
    // a string key.
    let s = p.get_mut(STRUCTURES).unwrap();
    *s = s.replace("n=\"errorType\"", "n=\"ErrorType\"").replace(
        "</rvStructures>",
        "<s t=\"_error\"><k n=\"errorType\" t=\"i\"/><k n=\"field\" t=\"s\"/></s></rvStructures>",
    );
    // A rich value may carry a fallback before its values.
    let v = p.get_mut(VALUES).unwrap();
    *v = v
        .replace(
            "<rv s=\"0\"><v>0</v><v>13</v>",
            "<rv s=\"0\"><fb t=\"e\">#VALUE!</fb><v>0</v><v>13</v>",
        )
        .replace(
            "<rv s=\"0\"><v>0</v><v>12</v><v>0</v><v>3</v></rv>",
            "<rv s=\"1\"><v>12</v><v>City</v></rv>",
        );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<c r=\"A1\" t=\"e\" vm=\"1\"><f>_xlfn._xlws.FILTER(C1:C1,FALSE)</f><v>#VALUE!</v>",
        "<c r=\"B1\" ><f>1+1</f><v>2</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    // A fallback after the values or twice, an errorType that is not an
    // integer, and a key indexing a property bag are not error tags.
    for (part, from, to) in [
        (
            VALUES,
            "<v>City</v></rv>",
            "<v>City</v><fb t=\"e\">#VALUE!</fb></rv>",
        ),
        (
            VALUES,
            "<fb t=\"e\">#VALUE!</fb>",
            "<fb t=\"e\">#VALUE!</fb><fb t=\"e\">#VALUE!</fb>",
        ),
        (
            STRUCTURES,
            "<k n=\"ErrorType\" t=\"i\"/>",
            "<k n=\"ErrorType\" t=\"s\"/>",
        ),
        (
            STRUCTURES,
            "<k n=\"field\" t=\"s\"/>",
            "<k n=\"field\" t=\"spb\"/>",
        ),
    ] {
        let mut bad = p.clone();
        let text = bad.get_mut(part).unwrap();
        assert!(text.contains(from), "{from} in {text}");
        *text = text.replace(from, to);
        reject(&bad);
    }
}
#[test]
fn extension_markup_outside_cells_is_not_workbook_or_cell_metadata() {
    let mut p = single("1+1", "<v>9</v>");
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace("</sheets>", "</sheets><extLst><ext uri=\"{140A7094-0E35-4892-8432-C4D2E57EDEB5}\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\"><x15:workbookPr chartTrackingRefBase=\"1\"/></ext></extLst>");
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace("</sheetData>", "</sheetData><extLst><ext uri=\"{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:dataValidations count=\"1\" xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:dataValidation type=\"list\"><x14:formula1><xm:f>Sheet1!$A$1:$A$2</xm:f></x14:formula1><xm:sqref>B1</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>");
    let input = pack(&p);
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
    assert!(member(&out.bytes, SHEET).contains("<xm:f>Sheet1!$A$1:$A$2</xm:f>"));
    // A non-empty foreign workbookPr would be read by Calamine as the epoch.
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "<x15:workbookPr chartTrackingRefBase=\"1\"/>",
        "<x15:workbookPr chartTrackingRefBase=\"1\"></x15:workbookPr>",
    );
    reject(&p);
    // Foreign lookalikes inside sheetData remain unsupported.
    let mut p = single("1+1", "<v>9</v>");
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace("<f>1+1</f>", "<f>1+1</f><x:v xmlns:x=\"urn:other\">3</x:v>");
    reject(&p);
}
#[test]
fn volatile_formulas_are_recalculated_not_refused() {
    let rows = "<row r=\"1\"><c r=\"A1\"><f>TODAY()</f><v>1</v></c></row><row r=\"2\"><c r=\"A2\"><f>1+1</f><v>9</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f>A1-A1+7</f><v>9</v></c></row><row r=\"4\"><c r=\"A4\"><f>SUBTOTAL(9,A2:A3)</f><v>0</v></c></row>";
    let out = recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default()).unwrap();
    assert!(matches!(data(&out.bytes, 0), Data::Float(n) if n > 45_000.0));
    assert_eq!(data(&out.bytes, 1), Data::Float(2.0));
    // Readers of volatile results are computed in the same pass.
    assert_eq!(data(&out.bytes, 2), Data::Float(7.0));
    assert_eq!(data(&out.bytes, 3), Data::Float(9.0));
}
#[test]
fn worksheet_tables_answer_structured_references() {
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Item</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>Qty</t></is></c><c r=\"D1\"><f>SUM(Sales[Qty])</f><v>0</v></c></row>\
        <row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>a</t></is></c><c r=\"B2\"><v>4</v></c><c r=\"D2\"><f>COUNTA(Sales[[#Headers],[Item]])</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>b</t></is></c><c r=\"B3\"><v>6</v></c></row>";
    let mut p = parts(rows);
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace(
        "</sheetData>",
        &format!("</sheetData><tableParts count=\"1\"><tablePart xmlns:r=\"{OFFICE}\" r:id=\"rId1\"/></tableParts>"),
    );
    p.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/table\" Target=\"../tables/table1.xml\"/></Relationships>"),
    );
    p.insert(
        "xl/tables/table1.xml".into(),
        format!("<table xmlns=\"{MAIN}\" id=\"1\" name=\"Table1\" displayName=\"Sales\" ref=\"A1:B3\" totalsRowShown=\"0\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Qty\"/></tableColumns></table>"),
    );
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>", "<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/></Types>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains("SUM(Sales[Qty])</f><v>10</v>"), "{sheet}");
    assert!(sheet.contains("[Item]])</f><v>1</v>"), "{sheet}");
    assert!(sheet.contains("<tablePart"));
}
#[test]
fn shared_formula_master_below_its_range_start_is_the_expansion_origin() {
    // The master (B2) is relative to itself although ref starts at B1.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>7</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><f t=\"shared\" ref=\"B1:B3\" si=\"0\">A2*10</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>0</v></c></row>";
    let input = pack(&parts(rows));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(
        sheet.contains("ref=\"B1:B3\" si=\"0\">A2*10</f><v>20</v>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<f t=\"shared\" si=\"0\"/><v>30</v>"),
        "{sheet}"
    );
    // A member above the master, read before it, also shifts from it.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f t=\"shared\" si=\"0\"/><v>0</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><f t=\"shared\" ref=\"B1:B3\" si=\"0\">A2*10</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>0</v></c></row>";
    let out = recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for (cell, value) in [("B1", 10), ("B3", 30)] {
        let written = format!("<c r=\"{cell}\"><f t=\"shared\" si=\"0\"/><v>{value}</v>");
        assert!(sheet.contains(&written), "{sheet}");
    }
    assert!(sheet.contains(">A2*10</f><v>20</v>"), "{sheet}");
}
#[test]
fn shared_formula_members_left_of_the_master_shift_from_the_master() {
    // Excel's shape: the group's first cell (B2) is the master and ref is the
    // group's bounding box A2:C3. A2 holds its own formula; A3 is a member
    // one column left of the master, so its relative column shifts by -1.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c><c r=\"C1\"><v>3</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><f>A1+1000</f><v>0</v></c><c r=\"B2\"><f t=\"shared\" ref=\"A2:C3\" si=\"0\">B$1*10+$D2</f><v>0</v></c><c r=\"C2\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"D2\"><v>5</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"C3\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"D3\"><v>7</v></c></row>";
    let out = recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default()).unwrap();
    assert_eq!(out.formula_cells, 6);
    let sheet = member(&out.bytes, SHEET);
    assert!(
        sheet.contains("<c r=\"A2\"><f>A1+1000</f><v>1001</v>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("ref=\"A2:C3\" si=\"0\">B$1*10+$D2</f><v>25</v>"),
        "{sheet}"
    );
    for (cell, value) in [("C2", 35), ("A3", 17), ("B3", 27), ("C3", 37)] {
        let written = format!("<c r=\"{cell}\"><f t=\"shared\" si=\"0\"/><v>{value}</v>");
        assert!(sheet.contains(&written), "{sheet}");
    }
    // Members and masters outside the declared ref are still refused.
    let rows = "<row r=\"2\"><c r=\"B2\"><f t=\"shared\" ref=\"B2:B3\" si=\"0\">1</f><v>0</v></c></row>\
        <row r=\"4\"><c r=\"B4\"><f t=\"shared\" si=\"0\"/><v>0</v></c></row>";
    reject(&parts(rows));
    let rows = "<row r=\"2\"><c r=\"B2\"><f t=\"shared\" ref=\"C2:C3\" si=\"0\">1</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"C3\"><f t=\"shared\" si=\"0\"/><v>0</v></c></row>";
    reject(&parts(rows));
}
#[test]
fn shared_formula_members_keep_quoted_sheet_names() {
    // Excel quotes sheet names that read as A1 references ('Q1', 'FY2024').
    // A member is the master at the member's place: its relative cells move,
    // the sheet name does not. Shifting 'Q1' would read Q2, and 'FY2024'
    // would name the missing FY2025 (or FX2025 left of the master).
    let rows = "<row r=\"1\"><c r=\"B1\"><f t=\"shared\" ref=\"B1:B2\" si=\"0\">'Q1'!A1*2</f><v>0</v></c></row>\
        <row r=\"2\"><c r=\"B2\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"D2\"><f t=\"shared\" ref=\"C2:D3\" si=\"1\">'FY2024'!B1+'Q1'!B1</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"C3\"><f t=\"shared\" si=\"1\"/><v>0</v></c><c r=\"D3\"><f t=\"shared\" si=\"1\"/><v>0</v></c></row>";
    let mut p = parts(rows);
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace("name=\"Sheet1\"", "name=\"Main\"");
    let others = [
        (
            "Q1",
            "<c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>30</v></c>",
            "<c r=\"A2\"><v>5</v></c><c r=\"B2\"><v>20</v></c>",
        ),
        (
            "Q2",
            "<c r=\"A1\"><v>100</v></c><c r=\"B1\"><v>3000</v></c>",
            "<c r=\"A2\"><v>500</v></c><c r=\"B2\"><v>2000</v></c>",
        ),
        (
            "FY2024",
            "<c r=\"B1\"><v>3</v></c>",
            "<c r=\"A2\"><v>7</v></c><c r=\"B2\"><v>9</v></c>",
        ),
    ];
    for (n, (name, row1, row2)) in others.into_iter().enumerate() {
        let id = n + 2;
        let wb = p.get_mut("xl/workbook.xml").unwrap();
        *wb = wb.replace(
            "</sheets>",
            &format!("<sheet name=\"{name}\" sheetId=\"{id}\" r:id=\"rId{id}\"/></sheets>"),
        );
        let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
        *rel = rel.replace("</Relationships>", &format!("<Relationship Id=\"rId{id}\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet{id}.xml\"/></Relationships>"));
        let ct = p.get_mut("[Content_Types].xml").unwrap();
        *ct = ct.replace("</Types>", &format!("<Override PartName=\"/xl/worksheets/sheet{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"));
        p.insert(
            format!("xl/worksheets/sheet{id}.xml"),
            format!("<worksheet xmlns=\"{MAIN}\"><sheetData><row r=\"1\">{row1}</row><row r=\"2\">{row2}</row></sheetData></worksheet>"),
        );
    }
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains(">'Q1'!A1*2</f><v>2</v>"), "{sheet}");
    assert!(
        sheet.contains(">'FY2024'!B1+'Q1'!B1</f><v>33</v>"),
        "{sheet}"
    );
    // B2 = 'Q1'!A2*2; C3 (left of the master) = 'FY2024'!A2+'Q1'!A2;
    // D3 = 'FY2024'!B2+'Q1'!B2.
    for (cell, si, value) in [("B2", 0, 10), ("C3", 1, 12), ("D3", 1, 29)] {
        let written = format!("<c r=\"{cell}\"><f t=\"shared\" si=\"{si}\"/><v>{value}</v>");
        assert!(sheet.contains(&written), "{sheet}");
    }
}
#[test]
fn multi_cell_array_formulas_write_results_into_their_extent() {
    // A1:A3 = 1,2,3. B1:B3 is a spilled dynamic array; C1:D2 a legacy CSE
    // array whose one-column result repeats across and pads with #N/A.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\" cm=\"1\"><f t=\"array\" ref=\"B1:B3\">A1:A3*10</f><v>0</v></c><c r=\"C1\"><f t=\"array\" ref=\"C1:D3\">A1:A2+1</f><v>0</v></c><c r=\"D1\"><v>0</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><v>0</v></c><c r=\"C2\"><v>0</v></c><c r=\"D2\"><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c><c r=\"B3\" t=\"str\"><v>old</v></c><c r=\"C3\"><v>0</v></c><c r=\"D3\"><v>0</v></c><c r=\"E3\"><f>SUM(B1:B3)</f><v>0</v></c></row>";
    let input = pack(&with_metadata(parts(rows), XLDAPR));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "A1:A3*10</f><v>10</v>",
        "<c r=\"B2\"><v>20</v>",
        // The removed type attribute leaves its separating space.
        "<c r=\"B3\" ><v>30</v>",
        "A1:A2+1</f><v>2</v>",
        "<c r=\"D1\"><v>2</v>",
        "<c r=\"C2\"><v>3</v>",
        "<c r=\"D2\"><v>3</v>",
        "<c r=\"C3\" t=\"e\"><v>#N/A</v>",
        "<c r=\"D3\" t=\"e\"><v>#N/A</v>",
        "SUM(B1:B3)</f><v>60</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
    // A formula inside another array formula's extent is not a member.
    let rows = "<row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A2\">{1;2}</f><v>1</v></c></row><row r=\"2\"><c r=\"A2\"><f>1</f><v>2</v></c></row>";
    reject(&parts(rows));
    for member in ["<f ca=\"1\">1</f>", "<f t=\"shared\" si=\"0\"/>"] {
        reject(&parts(&rows.replace("<f>1</f>", member)));
    }
}
#[test]
fn calculate_always_arrays_mark_each_member_with_an_empty_formula() {
    // Excel writes an empty <f ca="1"/> before the cache of every member of
    // an array formula whose anchor is calculated always (ca="1"), whatever
    // the member holds: B1:B4 is a legacy CSE array ("" , 20, 30, #N/A),
    // C1:C3 a dynamic array. D1:D3 is not calculated always: no member <f>.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\" t=\"str\"><f t=\"array\" ref=\"B1:B4\" ca=\"1\">IF(A1:A3&gt;1,A1:A3*10+0*TODAY(),&quot;&quot;)</f><v></v></c><c r=\"C1\" cm=\"1\"><f ca=\"1\" t=\"array\" ref=\"C1:C3\">OFFSET(A1,0,0,3)*2</f><v>2</v></c><c r=\"D1\"><f t=\"array\" ref=\"D1:D3\">A1:A3*3</f><v>3</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><v>20</v></c><c r=\"C2\"><v>4</v></c><c r=\"D2\"><v>6</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c><c r=\"B3\"><v>0</v></c><c r=\"C3\"><v>6</v></c><c r=\"D3\"><v>9</v></c></row>\
        <row r=\"4\"><c r=\"B4\" t=\"e\"><v>#N/A</v></c></row>";
    let input = pack(&with_metadata(parts(rows), XLDAPR));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<c r=\"B2\"><f ca=\"1\"/><v>20</v></c>",
        "<c r=\"B3\"><f ca=\"1\"/><v>30</v></c>",
        "<c r=\"B4\" t=\"e\"><f ca=\"1\"/><v>#N/A</v></c>",
        "<c r=\"C2\"><f ca=\"1\"/><v>4</v></c>",
        "<c r=\"C3\"><f ca=\"1\"/><v>6</v></c>",
        "<c r=\"D2\"><v>6</v></c>",
        "<c r=\"D3\"><v>9</v></c>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    // The anchors are untouched; five members gain a marker, B3 also a value.
    // cache_cells_changed counts patched caches only: the markers are not.
    assert!(sheet.contains("<f t=\"array\" ref=\"B1:B4\" ca=\"1\">"));
    assert_eq!(sheet.matches("<f ca=\"1\"/>").count(), 5);
    assert_eq!(out.cache_cells_changed, 1);
    // ca is an XML Schema boolean, so " true " is true as well.
    let padded = pack(&with_metadata(
        parts(&rows.replacen("ca=\"1\"", "ca=\" true \"", 1)),
        XLDAPR,
    ));
    let padded_sheet = member(
        &recalculate_xlsx_bytes(&padded, Default::default())
            .unwrap()
            .bytes,
        SHEET,
    );
    assert!(
        padded_sheet.contains("<c r=\"B2\"><f ca=\"1\"/><v>20</v></c>"),
        "{padded_sheet}"
    );
    // Excel's own markers are kept; recalculating again is an exact no-op.
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
    // A member without a cache receives its value after the marker; a member
    // left without one keeps it.
    let rows = "<row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A3\" ca=\"true\">ROW(INDIRECT(&quot;C1:C3&quot;))</f><v>1</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><f ca=\"1\"></f></c></row><row r=\"3\"><c r=\"A3\"><v>3</v></c></row>";
    let sheet = member(
        &recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default())
            .unwrap()
            .bytes,
        SHEET,
    );
    assert!(
        sheet.contains("<c r=\"A2\"><f ca=\"1\"></f><v>2</v></c>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<c r=\"A3\"><f ca=\"1\"/><v>3</v></c>"),
        "{sheet}"
    );
    // A namespace-prefixed worksheet gets a prefixed marker.
    let mut p = parts("");
    p.insert(
        SHEET.into(),
        format!("<x:worksheet xmlns:x=\"{MAIN}\"><x:sheetData><x:row r=\"1\"><x:c r=\"A1\"><x:f t=\"array\" ref=\"A1:A2\" ca=\"1\">{{1;2}}+0*RAND()</x:f><x:v>1</x:v></x:c></x:row><x:row r=\"2\"><x:c r=\"A2\"><x:v>2</x:v></x:c></x:row></x:sheetData></x:worksheet>"),
    );
    let sheet = member(
        &recalculate_xlsx_bytes(&pack(&p), Default::default())
            .unwrap()
            .bytes,
        SHEET,
    );
    assert!(
        sheet.contains("<x:c r=\"A2\"><x:f ca=\"1\"/><x:v>2</x:v></x:c>"),
        "{sheet}"
    );
}
/// Add a second worksheet, Sheet2, holding `rows`.
fn with_sheet2(mut p: BTreeMap<String, String>, rows: &str) -> BTreeMap<String, String> {
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</sheets>",
        "<sheet name=\"Sheet2\" sheetId=\"2\" r:id=\"rId2\"/></sheets>",
    );
    let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rel = rel.replace("</Relationships>", &format!("<Relationship Id=\"rId2\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>"));
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>", "<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>");
    p.insert(
        "xl/worksheets/sheet2.xml".into(),
        format!("<worksheet xmlns=\"{MAIN}\"><sheetData>{rows}</sheetData></worksheet>"),
    );
    p
}
fn with_names(mut p: BTreeMap<String, String>, names: &str) -> BTreeMap<String, String> {
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</workbook>",
        &format!("<definedNames>{names}</definedNames></workbook>"),
    );
    p
}
#[test]
fn volatile_formulas_and_their_readers_are_calculated_always() {
    // Excel saves ca="1" on a formula calling a volatile function (TODAY,
    // OFFSET, INDIRECT, ...) and on every formula reading such a cell, through
    // cell, range, name and table references and from other sheets. ca goes
    // after t and ref and before si, also on shared-formula descendants.
    let rows = "<row r=\"1\"><c r=\"A1\"><f>TODAY()</f><v>1</v></c><c r=\"B1\"><f>A1-A1+1</f><v>1</v></c><c r=\"C1\"><f>SUM(A1:B1)-A1</f><v>1</v></c><c r=\"D1\"><f>1+1</f><v>2</v></c>\
        <c r=\"E1\"><f t=\"shared\" ref=\"E1:E2\" si=\"0\">$D$1*2</f><v>4</v></c><c r=\"F1\"><f t=\"shared\" ref=\"F1:F2\" si=\"1\">$B$1*2</f><v>2</v></c>\
        <c r=\"G1\"><f>Stamp+1</f><v>2</v></c><c r=\"H1\"><f>Pick*3</f><v>6</v></c><c r=\"I1\"><f>INDIRECT(&quot;D1&quot;)</f><v>2</v></c></row>\
        <row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Qty</t></is></c><c r=\"E2\"><f t=\"shared\" si=\"0\"/><v>4</v></c><c r=\"F2\"><f t=\"shared\" si=\"1\"/><v>2</v></c><c r=\"G2\"><f>SUM(Sales[Qty])</f><v>1</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f>B1</f><v>1</v></c></row>";
    let mut p = with_names(
        with_sheet2(
            parts(rows),
            "<row r=\"1\"><c r=\"A1\"><f>Sheet1!C1*2</f><v>2</v></c><c r=\"B1\"><f>Sheet1!D1</f><v>2</v></c></row>",
        ),
        "<definedName name=\"Pick\">OFFSET(Sheet1!$D$1,0,0)</definedName><definedName name=\"Stamp\">Sheet1!$B$1</definedName>",
    );
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace(
        "</sheetData>",
        &format!("</sheetData><tableParts count=\"1\"><tablePart xmlns:r=\"{OFFICE}\" r:id=\"rId1\"/></tableParts>"),
    );
    p.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/table\" Target=\"../tables/table1.xml\"/></Relationships>"),
    );
    p.insert(
        "xl/tables/table1.xml".into(),
        format!("<table xmlns=\"{MAIN}\" id=\"1\" name=\"Sales\" displayName=\"Sales\" ref=\"A2:A3\" totalsRowShown=\"0\"><tableColumns count=\"1\"><tableColumn id=\"1\" name=\"Qty\"/></tableColumns></table>"),
    );
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>", "<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/></Types>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<c r=\"A1\"><f ca=\"1\">TODAY()</f>",
        "<f ca=\"1\">A1-A1+1</f><v>1</v>",
        "<f ca=\"1\">SUM(A1:B1)-A1</f><v>1</v>",
        "<f>1+1</f><v>2</v>",
        "<f t=\"shared\" ref=\"E1:E2\" si=\"0\">$D$1*2</f><v>4</v>",
        "<c r=\"E2\"><f t=\"shared\" si=\"0\"/><v>4</v>",
        "<f t=\"shared\" ref=\"F1:F2\" ca=\"1\" si=\"1\">$B$1*2</f><v>2</v>",
        "<c r=\"F2\"><f t=\"shared\" ca=\"1\" si=\"1\"/><v>2</v>",
        "<f ca=\"1\">Stamp+1</f><v>2</v>",
        "<f ca=\"1\">Pick*3</f><v>6</v>",
        "<f ca=\"1\">INDIRECT(&quot;D1&quot;)</f><v>2</v>",
        "<f ca=\"1\">SUM(Sales[Qty])</f><v>1</v>",
        "<c r=\"A3\"><f ca=\"1\">B1</f><v>1</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    let sheet2 = member(&out.bytes, "xl/worksheets/sheet2.xml");
    assert!(
        sheet2.contains("<f ca=\"1\">Sheet1!C1*2</f><v>2</v>"),
        "{sheet2}"
    );
    assert!(sheet2.contains("<f>Sheet1!D1</f><v>2</v>"), "{sheet2}");
    // Only TODAY's cache changed; the flags are not caches.
    assert_eq!(out.cache_cells_changed, 1);
    // The flags are kept and recalculating again is an exact no-op.
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
    // A false flag on a volatile formula becomes true; a flag the file has
    // is kept, and its readers are calculated always as well.
    let rows = "<row r=\"1\"><c r=\"A1\"><f ca=\"0\">INDIRECT(&quot;B1&quot;)</f><v>2</v></c><c r=\"B1\"><f ca=\"1\">1+1</f><v>2</v></c><c r=\"C1\"><f>B1*2</f><v>4</v></c></row>";
    let sheet = member(
        &recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default())
            .unwrap()
            .bytes,
        SHEET,
    );
    for expected in [
        "<f ca=\"1\">INDIRECT(&quot;B1&quot;)</f>",
        "<f ca=\"1\">1+1</f>",
        "<f ca=\"1\">B1*2</f>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
}
#[test]
fn unknown_functions_and_their_readers_are_calculated_always() {
    // A function Excel does not have (another application's function such as
    // Google Sheets' exported __xludf.DUMMYFUNCTION, a _xludf. user-defined
    // function, a bare unknown name) is #NAME? and calculated always, and so
    // are its readers. Functions written with Excel's _xlfn. prefix are known.
    // FORMULATEXT, and SUMIF whose sum range differs in size from its range,
    // are calculated always too.
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"str\"><f>IFERROR(__xludf.DUMMYFUNCTION(&quot;SORT(B1:B3)&quot;),&quot;x&quot;)</f><v>x</v></c><c r=\"B1\" t=\"str\"><f>A1&amp;&quot;y&quot;</f><v>xy</v></c>\
        <c r=\"C1\"><f>_xlfn.XOR(TRUE,FALSE)*1</f><v>1</v></c><c r=\"D1\"><f>SUMIF(E1:E2,1,F1:F3)</f><v>0</v></c><c r=\"E1\"><v>1</v></c><c r=\"F1\"><v>5</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><f>IFERROR(_xludf.IMAGE(&quot;u&quot;),3)</f><v>3</v></c><c r=\"B2\"><f>MYUDF(1)</f><v>0</v></c><c r=\"C2\"><f>SUM(C1,1)</f><v>2</v></c><c r=\"D2\"><f>SUMIF(E1:E2,1,F1:F2)</f><v>5</v></c><c r=\"E2\"><v>2</v></c><c r=\"F2\"><v>7</v></c></row>\
        <row r=\"3\"><c r=\"C3\" t=\"str\"><f>_xlfn.FORMULATEXT(C2)</f><v>=SUM(C1,1)</v></c><c r=\"F3\"><v>9</v></c></row>";
    let out = recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f ca=\"1\">IFERROR(__xludf.DUMMYFUNCTION(",
        "<f ca=\"1\">A1&amp;&quot;y&quot;</f><v>xy</v>",
        "<f>_xlfn.XOR(TRUE,FALSE)*1</f><v>1</v>",
        "<f ca=\"1\">SUMIF(E1:E2,1,F1:F3)</f><v>5</v>",
        "<f ca=\"1\">IFERROR(_xludf.IMAGE(&quot;u&quot;),3)</f><v>3</v>",
        "<c r=\"B2\" t=\"e\"><f ca=\"1\">MYUDF(1)</f><v>#NAME?</v>",
        "<f>SUM(C1,1)</f><v>2</v>",
        "<f>SUMIF(E1:E2,1,F1:F2)</f><v>5</v>",
        "<f ca=\"1\">_xlfn.FORMULATEXT(C2)</f>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn shared_formula_members_left_of_the_master_are_judged_on_their_own_formula() {
    // Excel's shared groups may have members left of their master (B2, ref
    // A2:C3; E2, ref D2:E3; G2, ref F2:G3). Each member is the master's
    // formula at the member's place, and its flag is that formula's: A3 is
    // A$1*2 and reads TODAY in A1 while B3 and C3 do not; every member of the
    // volatile E2 group is calculated always; K1 reads the member A3. G2's
    // SUMIF sums $J$5 over the one-cell H$5:H5, while its members F3 (G$5:G6)
    // and G3 (H$5:H6) resize it.
    let rows = "<row r=\"1\"><c r=\"A1\"><f>TODAY()</f><v>1</v></c><c r=\"B1\"><v>5</v></c><c r=\"C1\"><v>6</v></c><c r=\"K1\"><f>A3+1</f><v>0</v></c><c r=\"L1\"><f>B3+1</f><v>11</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><f>A1-A1+1000</f><v>1000</v></c><c r=\"B2\"><f t=\"shared\" ref=\"A2:C3\" si=\"0\">B$1*2</f><v>10</v></c><c r=\"C2\"><f t=\"shared\" si=\"0\"/><v>12</v></c>\
        <c r=\"E2\"><f t=\"shared\" ref=\"D2:E3\" si=\"1\">ROW()+0*NOW()</f><v>2</v></c><c r=\"G2\"><f t=\"shared\" ref=\"F2:G3\" si=\"2\">SUMIF(H$5:H5,\"&gt;0\",$J$5)</f><v>10</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f t=\"shared\" si=\"0\"/><v>0</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>10</v></c><c r=\"C3\"><f t=\"shared\" si=\"0\"/><v>12</v></c>\
        <c r=\"D3\"><f t=\"shared\" si=\"1\"/><v>3</v></c><c r=\"E3\"><f t=\"shared\" si=\"1\"/><v>3</v></c><c r=\"F3\"><f t=\"shared\" si=\"2\"/><v>0</v></c><c r=\"G3\"><f t=\"shared\" si=\"2\"/><v>10</v></c></row>\
        <row r=\"5\"><c r=\"H5\"><v>1</v></c><c r=\"J5\"><v>10</v></c></row>";
    let out = recalculate_xlsx_bytes(&pack(&parts(rows)), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<c r=\"A1\"><f ca=\"1\">TODAY()</f>",
        "<c r=\"A2\"><f ca=\"1\">A1-A1+1000</f><v>1000</v>",
        "<c r=\"B2\"><f t=\"shared\" ref=\"A2:C3\" si=\"0\">B$1*2</f><v>10</v>",
        "<c r=\"C2\"><f t=\"shared\" si=\"0\"/><v>12</v>",
        "<c r=\"A3\"><f t=\"shared\" ca=\"1\" si=\"0\"/>",
        "<c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>10</v>",
        "<c r=\"C3\"><f t=\"shared\" si=\"0\"/><v>12</v>",
        "<c r=\"K1\"><f ca=\"1\">A3+1</f>",
        "<c r=\"L1\"><f>B3+1</f><v>11</v>",
        "<c r=\"E2\"><f t=\"shared\" ref=\"D2:E3\" ca=\"1\" si=\"1\">ROW()+0*NOW()</f><v>2</v>",
        "<c r=\"D3\"><f t=\"shared\" ca=\"1\" si=\"1\"/><v>3</v>",
        "<c r=\"E3\"><f t=\"shared\" ca=\"1\" si=\"1\"/><v>3</v>",
        "<c r=\"G2\"><f t=\"shared\" ref=\"F2:G3\" si=\"2\">SUMIF(H$5:H5,\"&gt;0\",$J$5)</f><v>10</v>",
        "<c r=\"F3\"><f t=\"shared\" ca=\"1\" si=\"2\"/><v>0</v>",
        "<c r=\"G3\"><f t=\"shared\" ca=\"1\" si=\"2\"/><v>10</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn arrays_reading_calculated_always_cells_mark_the_members_that_read_them() {
    // An array formula that only reads calculated-always cells is calculated
    // always, and Excel marks with <f ca="1"/> the members whose elements of
    // a range as tall (or wide) as the array are calculated always: B2 reads
    // A2, B3 reads the value A3 and B4 the empty A4. A reference of another
    // shape (D1:D3 reads A1) reaches every member, as does a volatile array
    // formula (E1:E2) and one the file flags without reading a flagged cell
    // (G1:G2). The array formula F1:F2 reads nothing calculated always.
    let rows = "<row r=\"1\"><c r=\"A1\"><f>IFERROR(__xludf.DUMMYFUNCTION(&quot;x&quot;),1)</f><v>1</v></c><c r=\"B1\"><f t=\"array\" ref=\"B1:B4\">IF(ISNUMBER(A1:A4),A1:A4*2,&quot;&quot;)</f><v>2</v></c>\
        <c r=\"C1\"><f>SUM(B1:B4)</f><v>16</v></c><c r=\"D1\"><f t=\"array\" ref=\"D1:D3\">A1*{1;2;3}</f><v>1</v></c><c r=\"E1\"><f t=\"array\" ref=\"E1:E2\">ROW(INDIRECT(&quot;A1:A2&quot;))</f><v>1</v></c><c r=\"F1\"><f t=\"array\" ref=\"F1:F2\">{1;2}*3</f><v>3</v></c><c r=\"G1\"><f t=\"array\" ref=\"G1:G2\" ca=\"1\">{1;2}*4</f><v>4</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><f>IFERROR(__xludf.DUMMYFUNCTION(&quot;x&quot;),2)</f><v>2</v></c><c r=\"B2\"><v>4</v></c><c r=\"C2\"><f>B3+1</f><v>11</v></c><c r=\"D2\"><v>2</v></c><c r=\"E2\"><v>2</v></c><c r=\"F2\"><v>6</v></c><c r=\"G2\"><v>8</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>5</v></c><c r=\"B3\"><v>10</v></c><c r=\"C3\"><f>B2+1</f><v>5</v></c><c r=\"D3\"><v>3</v></c></row>\
        <row r=\"4\"><c r=\"B4\" t=\"str\"><v></v></c></row>";
    let input = pack(&parts(rows));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f t=\"array\" ref=\"B1:B4\" ca=\"1\">",
        "<c r=\"B2\"><f ca=\"1\"/><v>4</v></c>",
        "<c r=\"B3\"><v>10</v></c>",
        "<c r=\"B4\" t=\"str\"><v></v></c>",
        "<f ca=\"1\">SUM(B1:B4)</f><v>16</v>",
        "<f>B3+1</f><v>11</v>",
        "<f ca=\"1\">B2+1</f><v>5</v>",
        "<f t=\"array\" ref=\"D1:D3\" ca=\"1\">",
        "<c r=\"D2\"><f ca=\"1\"/><v>2</v></c>",
        "<c r=\"D3\"><f ca=\"1\"/><v>3</v></c>",
        "<f t=\"array\" ref=\"E1:E2\" ca=\"1\">",
        "<c r=\"E2\"><f ca=\"1\"/><v>2</v></c>",
        "<f t=\"array\" ref=\"F1:F2\">{1;2}*3</f><v>3</v>",
        "<c r=\"F2\"><v>6</v></c>",
        "<c r=\"G2\"><f ca=\"1\"/><v>8</v></c>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(out.cache_cells_changed, 0);
    let again = recalculate_xlsx_bytes(&out.bytes, Default::default()).unwrap();
    assert_eq!(member(&again.bytes, SHEET), sheet);
    assert_eq!(again.bytes, out.bytes);
}
#[test]
fn workbooks_without_calculated_always_formulas_are_unchanged() {
    // Nothing volatile or unknown: ranges, names, shared and array formulas
    // with current caches are an exact no-op.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f>SUM(A1:A2)</f><v>3</v></c><c r=\"C1\"><f t=\"shared\" ref=\"C1:C2\" si=\"0\">A1*Two</f><v>2</v></c><c r=\"D1\"><f t=\"array\" ref=\"D1:D2\">A1:A2*10</f><v>10</v></c><c r=\"E1\"><f>SUMIF(A1:A2,\"&gt;1\",A1:A2)</f><v>2</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"C2\"><f t=\"shared\" si=\"0\"/><v>4</v></c><c r=\"D2\"><v>20</v></c><c r=\"E2\"><f>INDEX(A1:A2,2)+SUBTOTAL(9,A1:A2)</f><v>5</v></c></row>";
    let input = pack(&with_names(
        parts(rows),
        "<definedName name=\"Two\">Sheet1!$A$2</definedName>",
    ));
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.bytes, input);
    assert_eq!(out.cache_cells_changed, 0);
}
/// Add a worksheet table `name` over `area` of Sheet1, one column per name
/// in `columns`.
fn with_table(
    mut p: BTreeMap<String, String>,
    name: &str,
    area: &str,
    columns: &[&str],
) -> BTreeMap<String, String> {
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace(
        "</sheetData>",
        &format!("</sheetData><tableParts count=\"1\"><tablePart xmlns:r=\"{OFFICE}\" r:id=\"rId1\"/></tableParts>"),
    );
    p.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/table\" Target=\"../tables/table1.xml\"/></Relationships>"),
    );
    let names: String = columns
        .iter()
        .enumerate()
        .map(|(i, column)| format!("<tableColumn id=\"{}\" name=\"{column}\"/>", i + 1))
        .collect();
    p.insert(
        "xl/tables/table1.xml".into(),
        format!("<table xmlns=\"{MAIN}\" id=\"1\" name=\"{name}\" displayName=\"{name}\" ref=\"{area}\" totalsRowShown=\"0\"><tableColumns count=\"{}\">{names}</tableColumns></table>", columns.len()),
    );
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>", "<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/></Types>");
    p
}
#[test]
fn arrays_reading_through_a_name_mark_each_member_reading_a_flagged_cell() {
    // A reference through a defined name is a reference to the name's
    // cells, so {=Data*2} lines up with A1:A3 as {=A1:A3*2} does: B3's
    // element reads A3, which reads TODAY in A1, and is marked like the
    // anchor's; B2's reads the plain A2. A formula reading the name is
    // calculated always too, and recalculating the output changes nothing.
    let rows = "<row r=\"1\"><c r=\"A1\"><f>TODAY()</f><v>1</v></c><c r=\"B1\"><f t=\"array\" ref=\"B1:B3\">Data*2</f><v>2</v></c><c r=\"C1\"><f>SUM(Data)</f><v>8</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>5</v></c><c r=\"B2\"><v>10</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f>A1+1</f><v>2</v></c><c r=\"B3\"><v>4</v></c></row>";
    let named = with_names(
        parts(rows),
        "<definedName name=\"Data\">Sheet1!$A$1:$A$3</definedName>",
    );
    let direct = parts(&rows.replace("Data", "A1:A3"));
    for (p, array) in [(named, "Data*2"), (direct, "A1:A3*2")] {
        let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
        let sheet = member(&out.bytes, SHEET);
        for expected in [
            "<c r=\"A1\"><f ca=\"1\">TODAY()</f>",
            &format!("<f t=\"array\" ref=\"B1:B3\" ca=\"1\">{array}</f>"),
            "<c r=\"B2\"><v>10</v></c>",
            "<c r=\"B3\"><f ca=\"1\"/>",
            "<c r=\"A3\"><f ca=\"1\">A1+1</f>",
            "<f ca=\"1\">SUM(",
        ] {
            assert!(sheet.contains(expected), "{expected} in {sheet}");
        }
        assert_eq!(
            recalculate_xlsx_bytes(&out.bytes, Default::default())
                .unwrap()
                .bytes,
            out.bytes
        );
    }
}
#[test]
fn readers_through_names_defined_by_formulas_are_calculated_always() {
    // The cells a name's formula refers to are precedents of every formula
    // using the name: an expression over A1 (Twice), a name defined as a
    // range name (Alias), a range ending at INDEX over a column holding the
    // TODAY reader G2 (Dyn) and a table column holding one (Qty). Plain's
    // formula reads nothing calculated always.
    let rows = "<row r=\"1\"><c r=\"A1\"><f>TODAY()</f><v>1</v></c><c r=\"B1\"><f>Twice+1</f><v>3</v></c><c r=\"C1\"><f>SUM(Alias)</f><v>8</v></c><c r=\"D1\"><f>SUM(Dyn)</f><v>3</v></c>\
        <c r=\"E1\"><f>SUM(Qty)</f><v>1</v></c><c r=\"F1\"><v>7</v></c><c r=\"G1\"><v>1</v></c><c r=\"H1\" t=\"inlineStr\"><is><t>Q</t></is></c><c r=\"I1\"><f>Plain*2</f><v>14</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>7</v></c><c r=\"G2\"><f>TODAY()*0+2</f><v>2</v></c><c r=\"H2\"><f>TODAY()*0+1</f><v>1</v></c></row>";
    let p = with_names(
        with_table(parts(rows), "Sales", "H1:H2", &["Q"]),
        "<definedName name=\"Twice\">Sheet1!$A$1*2</definedName><definedName name=\"Base\">Sheet1!$A$1:$A$2</definedName><definedName name=\"Alias\">Base</definedName>\
         <definedName name=\"Dyn\">Sheet1!$G$1:INDEX(Sheet1!$G:$G,2)</definedName><definedName name=\"Qty\">Sales[Q]</definedName><definedName name=\"Plain\">Sheet1!$F$1*1</definedName>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f ca=\"1\">Twice+1</f>",
        "<f ca=\"1\">SUM(Alias)</f>",
        "<f ca=\"1\">SUM(Dyn)</f><v>3</v>",
        "<f ca=\"1\">SUM(Qty)</f><v>1</v>",
        "<f>Plain*2</f><v>14</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn formulas_use_the_defined_name_their_sheet_sees() {
    // A formula resolves its sheet's own name before the workbook's
    // (ECMA-376 18.2.5, localSheetId). Sheet1's X is the plain A1 and
    // Sheet2's is NOW(), so only the formulas meaning Sheet2's X are
    // calculated always, Sheet2!X on Sheet1 included.
    let sheet2 = "<row r=\"1\"><c r=\"A1\"><f>X</f><v>1</v></c></row>";
    let rows = "<row r=\"1\"><c r=\"A1\"><v>5</v></c><c r=\"B1\"><f>X+1</f><v>6</v></c><c r=\"C1\"><f>Sheet2!X</f><v>1</v></c></row>";
    for names in [
        "<definedName name=\"X\" localSheetId=\"0\">Sheet1!$A$1</definedName><definedName name=\"X\" localSheetId=\"1\">NOW()</definedName>",
        "<definedName name=\"X\">Sheet1!$A$1</definedName><definedName name=\"X\" localSheetId=\"1\">NOW()</definedName>",
    ] {
        let p = with_names(with_sheet2(parts(rows), sheet2), names);
        let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
        let sheet = member(&out.bytes, SHEET);
        assert!(sheet.contains("<f>X+1</f><v>6</v>"), "{names}: {sheet}");
        assert!(
            sheet.contains("<f ca=\"1\">Sheet2!X</f>"),
            "{names}: {sheet}"
        );
        let sheet2 = member(&out.bytes, "xl/worksheets/sheet2.xml");
        assert!(sheet2.contains("<f ca=\"1\">X</f>"), "{names}: {sheet2}");
    }
    // A workbook-level NOW() shadowed on Sheet1 only reaches Sheet2.
    let p = with_names(
        with_sheet2(parts(rows), sheet2),
        "<definedName name=\"X\">NOW()</definedName><definedName name=\"X\" localSheetId=\"0\">Sheet1!$A$1</definedName>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains("<f>X+1</f><v>6</v>"), "{sheet}");
    assert!(sheet.contains("<f ca=\"1\">Sheet2!X</f>"), "{sheet}");
    let sheet2 = member(&out.bytes, "xl/worksheets/sheet2.xml");
    assert!(sheet2.contains("<f ca=\"1\">X</f>"), "{sheet2}");
}
#[test]
fn sumif_resizing_is_judged_on_the_ranges_each_cell_evaluates() {
    // SUMIF and AVERAGEIF sum the criteria range's shape from the sum
    // range's top-left cell. The shared B1:B3 grows its criteria range
    // row by row, so B2 and B3 resize $C$1 and B1 does not. Ranges through a
    // name (Crit), INDEX and a table column are sized as they evaluate.
    let rows = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f t=\"shared\" ref=\"B1:B3\" si=\"0\">SUMIF($A$1:A1,\"&gt;0\",$C$1)</f><v>10</v></c><c r=\"C1\"><v>10</v></c>\
        <c r=\"D1\"><f>SUMIF(Crit,\"&gt;0\",$C$1)</f><v>60</v></c><c r=\"E1\"><f>SUMIF($A$1:$A$3,\"&gt;0\",INDEX($C:$C,1):INDEX($C:$C,3))</f><v>60</v></c><c r=\"F1\" t=\"inlineStr\"><is><t>N</t></is></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><f t=\"shared\" si=\"0\"/><v>30</v></c><c r=\"C2\"><v>20</v></c>\
        <c r=\"D2\"><f>SUMIF(Crit,\"&gt;0\",$C$1:$C$3)</f><v>60</v></c><c r=\"E2\"><f>SUMIF($A$1:$A$3,\"&gt;0\",INDEX($C:$C,1):INDEX($C:$C,1))</f><v>60</v></c><c r=\"F2\"><v>1</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>3</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\"/><v>60</v></c><c r=\"C3\"><v>30</v></c>\
        <c r=\"D3\"><f>AVERAGEIF(Sales[N],\"&gt;0\",$C$1)</f><v>15</v></c><c r=\"E3\"><f>SUMIF(Sales[N],\"&gt;0\",$C$1:$C$2)</f><v>30</v></c><c r=\"F3\"><v>2</v></c></row>";
    let p = with_names(
        with_table(parts(rows), "Sales", "F1:F3", &["N"]),
        "<definedName name=\"Crit\">Sheet1!$A$1:$A$3</definedName>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f t=\"shared\" ref=\"B1:B3\" si=\"0\">SUMIF($A$1:A1,\"&gt;0\",$C$1)</f><v>10</v>",
        "<c r=\"B2\"><f t=\"shared\" ca=\"1\" si=\"0\"/><v>30</v>",
        "<c r=\"B3\"><f t=\"shared\" ca=\"1\" si=\"0\"/><v>60</v>",
        "<f ca=\"1\">SUMIF(Crit,",
        "<f>SUMIF(Crit,\"&gt;0\",$C$1:$C$3)</f><v>60</v>",
        "<f>SUMIF($A$1:$A$3,\"&gt;0\",INDEX($C:$C,1):INDEX($C:$C,3))</f><v>60</v>",
        "<f ca=\"1\">SUMIF($A$1:$A$3,\"&gt;0\",INDEX($C:$C,1):INDEX($C:$C,1))</f><v>60</v>",
        "<f ca=\"1\">AVERAGEIF(Sales[N],",
        "<f>SUMIF(Sales[N],\"&gt;0\",$C$1:$C$2)</f><v>30</v>",
    ] {
        assert!(sheet.contains(expected), "{expected} in {sheet}");
    }
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn multiple_changed_members_relocate_growing_and_shrinking_payloads() {
    let old = (0..2048u32)
        .map(|n| format!("{:08x}", n.wrapping_mul(2_654_435_761)))
        .collect::<String>();
    let mut p = single("1+1", &format!("<v>{old}</v>"));
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace("r=\"A1\"", "r=\"A1\" t=\"str\"");
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</sheets>",
        "<sheet name=\"Sheet2\" sheetId=\"2\" r:id=\"rId2\"/></sheets>",
    );
    let rel = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rel=rel.replace("</Relationships>",&format!("<Relationship Id=\"rId2\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>"));
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct=ct.replace("</Types>","<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>");
    p.insert("xl/worksheets/sheet2.xml".into(),format!("<worksheet xmlns=\"{MAIN}\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"str\"><f>REPT(&quot;x&quot;,32767)</f><v>q</v></c></row></sheetData></worksheet>"));
    p.insert("zz/opaque.bin".into(), "opaque tail".into());
    let input = pack(&p);
    let out = recalculate_xlsx_bytes(&input, Default::default()).unwrap();
    assert_eq!(out.worksheet_parts_changed, 2);
    let mut before = ZipArchive::new(Cursor::new(&input)).unwrap();
    let mut after = ZipArchive::new(Cursor::new(&out.bytes)).unwrap();
    assert!(
        before.by_name(SHEET).unwrap().compressed_size()
            > after.by_name(SHEET).unwrap().compressed_size()
    );
    assert!(
        before
            .by_name("xl/worksheets/sheet2.xml")
            .unwrap()
            .compressed_size()
            < after
                .by_name("xl/worksheets/sheet2.xml")
                .unwrap()
                .compressed_size()
    );
    let a = before.by_name("zz/opaque.bin").unwrap();
    let b = after.by_name("zz/opaque.bin").unwrap();
    assert_eq!(
        &input[a.data_start() as usize..(a.data_start() + a.compressed_size()) as usize],
        &out.bytes[b.data_start() as usize..(b.data_start() + b.compressed_size()) as usize]
    );
    assert_eq!(data(&out.bytes, 0), Data::Float(2.0));
    assert_eq!(
        recalculate_xlsx_bytes(&out.bytes, Default::default())
            .unwrap()
            .bytes,
        out.bytes
    );
}
#[test]
fn duplicate_zip_names_are_not_hidden_by_archive_index() {
    let mut input = fixture("1+1", "99");
    let (headers, footer) = directory(&input);
    let a = headers[0];
    let len = 46 + h16(&input, a + 28) + h16(&input, a + 30) + h16(&input, a + 32);
    let copy = input[a..a + len].to_vec();
    input.splice(footer..footer, copy);
    let footer = footer + len;
    for offset in [8, 10] {
        input[footer + offset..footer + offset + 2]
            .copy_from_slice(&((headers.len() + 1) as u16).to_le_bytes());
    }
    let size = h32(&input, footer + 12) + len;
    input[footer + 12..footer + 16].copy_from_slice(&(size as u32).to_le_bytes());
    assert!(recalculate_xlsx_bytes(&input, Default::default()).is_err());
}
#[test]
fn data_descriptors_and_inconsistent_headers_are_rejected() {
    let original = fixture("1+1", "99");
    let (headers, _) = directory(&original);
    let a = headers[0];
    let local = h32(&original, a + 42);
    let mut mismatch = original.clone();
    mismatch[local + 14] ^= 1;
    assert!(recalculate_xlsx_bytes(&mismatch, Default::default()).is_err());
    let mut descriptor = original;
    descriptor[a + 8] |= 8;
    descriptor[local + 6] |= 8;
    assert!(recalculate_xlsx_bytes(&descriptor, Default::default()).is_err());
}
#[test]
fn actual_expansion_and_output_limits_are_enforced() {
    let mut p = single("1+1", "<v>99</v>");
    p.insert("custom/opaque.bin".into(), "x".repeat(1 << 20));
    let input = pack(&p);
    let mut o = XlsxRecalculateOptions::default();
    o.limits.max_expanded_bytes = 1 << 16;
    assert!(recalculate_xlsx_bytes(&input, o).is_err());
    for cache in ["2", "99"] {
        let mut o = XlsxRecalculateOptions::default();
        o.limits.max_output_bytes = 1;
        assert!(recalculate_xlsx_bytes(&fixture("1+1", cache), o).is_err());
    }
    let mut p = single("1+1", "<v>99</v>");
    let xml = p.get_mut(SHEET).unwrap();
    *xml = xml.replace("<sheetData>", "<dimension ref=\"A1:XFD1\"/><sheetData>");
    let mut o = XlsxRecalculateOptions::default();
    o.limits.max_columns = 256;
    assert!(recalculate_xlsx_bytes(&pack(&p), o).is_err());
    assert!(recalculate_xlsx_bytes(&pack(&p), Default::default()).is_ok());
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_native_output_and_permissions() {
    use formualizer_workbook::recalculate_xlsx_file;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.xlsx");
    let output = dir.path().join("out.xlsx");
    std::fs::write(&input, b"bad input").unwrap();
    std::fs::write(&output, b"existing output").unwrap();
    assert!(recalculate_xlsx_file(&input, Some(&output), Default::default()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"existing output");
    let source = fixture("1+1", "99");
    std::fs::write(&input, &source).unwrap();
    let out = recalculate_xlsx_file(&input, Some(&output), Default::default()).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), out.bytes);
    assert_eq!(std::fs::read(&input).unwrap(), source);
    recalculate_xlsx_file(&input, None, Default::default()).unwrap();
    assert_eq!(data(&std::fs::read(&input).unwrap(), 0), Data::Float(2.0));
}
#[test]
fn linked_workbook_references_read_saved_values() {
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><f>[1]Rates!$B$2*2</f><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><f>VLOOKUP(\"pear\",[1]Rates!A1:B3,2,FALSE)</f><v>0</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><f>SUMIF([1]Rates!A1:A3,\"pear\",[1]Rates!B1:B3)</f><v>0</v></c></row>\
         <row r=\"4\"><c r=\"A4\"><f>IFERROR([1]Gone!A1,\"missing\")</f><v>0</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</sheets>",
        "</sheets><externalReferences><externalReference r:id=\"rId9\"/></externalReferences>",
    );
    let rels = p.get_mut("xl/_rels/workbook.xml.rels").unwrap();
    *rels = rels.replace(
        "</Relationships>",
        &format!("<Relationship Id=\"rId9\" Type=\"{OFFICE}/externalLink\" Target=\"externalLinks/externalLink1.xml\"/></Relationships>"),
    );
    let types = p.get_mut("[Content_Types].xml").unwrap();
    *types = types.replace(
        "</Types>",
        "<Override PartName=\"/xl/externalLinks/externalLink1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml\"/></Types>",
    );
    let link = format!(
        "<externalLink xmlns=\"{MAIN}\" xmlns:r=\"{OFFICE}\"><externalBook r:id=\"rId1\"><sheetNames><sheetName val=\"Rates\"/></sheetNames><sheetDataSet><sheetData sheetId=\"0\" refreshError=\"1\"><row r=\"1\"><cell r=\"A1\" t=\"str\"><v>apple</v></cell><cell r=\"B1\"><v>3</v></cell></row><row r=\"2\"><cell r=\"A2\" t=\"str\"><v>pear</v></cell><cell r=\"B2\"><v>5</v></cell></row></sheetData></sheetDataSet></externalBook></externalLink>"
    );
    p.insert("xl/externalLinks/externalLink1.xml".into(), link.clone());
    p.insert(
        "xl/externalLinks/_rels/externalLink1.xml.rels".into(),
        format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/externalLinkPath\" Target=\"Rates.xlsx\" TargetMode=\"External\"/></Relationships>"),
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    assert_eq!(data(&out.bytes, 0), Data::Float(10.0));
    assert_eq!(data(&out.bytes, 1), Data::Float(5.0));
    // Range-only parameters cannot read a closed linked workbook.
    assert!(matches!(data(&out.bytes, 2), Data::Error(_)));
    assert_eq!(data(&out.bytes, 3), Data::String("missing".into()));
    assert_eq!(
        member(&out.bytes, "xl/externalLinks/externalLink1.xml"),
        link
    );
}
#[test]
fn excel_width_worksheets_recalculate() {
    // Excel's grid is 16,384 columns wide (A..XFD).
    let rows = "<row r=\"1\"><c r=\"A1\"><v>2</v></c><c r=\"XFC1\"><f>A1*3</f><v>0</v></c><c r=\"XFD1\"><f>XFC1+1</f><v>0</v></c></row>";
    let mut p = parts(rows);
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace("<sheetData>", "<dimension ref=\"A1:XFD1\"/><sheetData>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let mut x = Xlsx::new(Cursor::new(&out.bytes)).unwrap();
    let range = x.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.get_value((0, 16_382)), Some(&Data::Float(6.0)));
    assert_eq!(range.get_value((0, 16_383)), Some(&Data::Float(7.0)));
}
#[test]
fn structured_references_wait_for_formulas_in_the_table() {
    // D1:F1 read table columns whose cells are formulas stored later in the
    // sheet; Share reads another column of its own table.
    let rows = "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Item</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>Qty</t></is></c><c r=\"C1\" t=\"inlineStr\"><is><t>Share</t></is></c><c r=\"D1\"><f>SUM(Sales[Qty])</f><v>0</v></c><c r=\"E1\"><f>MAX(Sales[Qty])</f><v>0</v></c><c r=\"F1\"><f>SUM(Sales[Share])</f><v>0</v></c></row>\
        <row r=\"2\"><c r=\"A2\"><v>4</v></c><c r=\"B2\"><f>A2*1</f><v>0</v></c><c r=\"C2\"><f>B2/SUM(Sales[Qty])</f><v>0</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><v>6</v></c><c r=\"B3\"><f>A3*1</f><v>0</v></c><c r=\"C3\"><f>B3/SUM(Sales[Qty])</f><v>0</v></c></row>";
    let mut p = parts(rows);
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace(
        "</sheetData>",
        &format!("</sheetData><tableParts count=\"1\"><tablePart xmlns:r=\"{OFFICE}\" r:id=\"rId1\"/></tableParts>"),
    );
    p.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/table\" Target=\"../tables/table1.xml\"/></Relationships>"),
    );
    p.insert(
        "xl/tables/table1.xml".into(),
        format!("<table xmlns=\"{MAIN}\" id=\"1\" name=\"Table1\" displayName=\"Sales\" ref=\"A1:C3\" totalsRowShown=\"0\"><tableColumns count=\"3\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Qty\"/><tableColumn id=\"3\" name=\"Share\"/></tableColumns></table>"),
    );
    let ct = p.get_mut("[Content_Types].xml").unwrap();
    *ct = ct.replace("</Types>", "<Override PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/></Types>");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "SUM(Sales[Qty])</f><v>10</v>",
        "MAX(Sales[Qty])</f><v>6</v>",
        "SUM(Sales[Share])</f><v>1</v>",
        "B2/SUM(Sales[Qty])</f><v>0.4</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
    // A calculated column that looks up its own whole table reads the other
    // columns; it is not a circular reference.
    let mut p = p;
    let sheet = p.get_mut(SHEET).unwrap();
    *sheet = sheet.replace("B2/SUM(Sales[Qty])", "VLOOKUP(A3,Sales[],2,FALSE)");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(
        sheet.contains("VLOOKUP(A3,Sales[],2,FALSE)</f><v>6</v>"),
        "{sheet}"
    );
}
#[test]
fn defined_names_keep_spaces_around_entities_in_sheet_names() {
    // 'A &amp; B'!$A$2 is split by the entity into text events; the spaces
    // around it are part of the sheet name.
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><f>Yr+1</f><v>0</v></c><c r=\"B1\"><f>\"1-JAN\"&amp;Yr</f><v>0</v></c></row><row r=\"2\"><c r=\"A2\"><f>2000+26</f><v>0</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb
        .replace(
            "</workbook>",
            "<definedNames><definedName name=\"Yr\">'A &amp; B'!$A$2</definedName></definedNames></workbook>",
        )
        .replace("name=\"Sheet1\"", "name=\"A &amp; B\"");
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains("<f>Yr+1</f><v>2027</v>"), "{sheet}");
    assert!(sheet.contains("<v>1-JAN2026</v>"), "{sheet}");
}
#[test]
fn names_defined_by_formulas_and_constants() {
    // Names may hold a formula (MATCH over other names), an array constant
    // or a reference-returning formula (OFFSET); each evaluates where used.
    // The formulas reading the OFFSET name are calculated always.
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>2020</v></c><c r=\"B1\" t=\"inlineStr\"><is><t>April</t></is></c><c r=\"C1\" t=\"inlineStr\"><is><t>Monday</t></is></c></row>\
         <row r=\"2\"><c r=\"A2\"><f>MonOpt</f><v>0</v></c><c r=\"B2\"><f>WkOpt</f><v>0</v></c><c r=\"C2\"><f>SUM(Days)</f><v>0</v></c><c r=\"D2\"><f>WEEKDAY(DATE(Yr,MonOpt,1),WkOpt)</f><v>0</v></c><c r=\"E2\"><f>INDEX(Days+1,2)</f><v>0</v></c><c r=\"F2\"><f>DATE(Yr,MonOpt,1)</f><v>0</v></c></row>\
         <row r=\"3\"><c r=\"G3\"><v>5</v></c><c r=\"H3\"><f>SUM(Filled)</f><v>0</v></c><c r=\"I3\"><f>ROWS(Filled)</f><v>0</v></c><c r=\"J3\"><f>MATCH(7,Filled,0)</f><v>0</v></c><c r=\"K3\"><f>INDEX(Filled,2)</f><v>0</v></c><c r=\"L3\"><f>COUNTIF(Filled,\"&gt;5\")</f><v>0</v></c></row>\
         <row r=\"4\"><c r=\"G4\"><v>7</v></c></row><row r=\"5\"><c r=\"G5\"><v>9</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</workbook>",
        "<definedNames><definedName name=\"Days\">{0,1,2,3,4,5,6}</definedName><definedName name=\"Filled\">OFFSET(Sheet1!$G$3,0,0,COUNT(Sheet1!$G:$G),1)</definedName><definedName name=\"MonOpt\">MATCH(Mon,Months,0)</definedName><definedName name=\"Mon\">Sheet1!$B$1</definedName><definedName name=\"Months\">{\"January\",\"February\",\"March\",\"April\"}</definedName><definedName name=\"Yr\">Sheet1!$A$1</definedName><definedName name=\"WkOpt\">MATCH(WS,Weekdays,0)+10</definedName><definedName name=\"WS\">Sheet1!$C$1</definedName><definedName name=\"Weekdays\">{\"Monday\",\"Tuesday\"}</definedName></definedNames></workbook>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f>MonOpt</f><v>4</v>",
        "<f>WkOpt</f><v>11</v>",
        "<f>SUM(Days)</f><v>21</v>",
        "<f>WEEKDAY(DATE(Yr,MonOpt,1),WkOpt)</f><v>3</v>",
        "<f>INDEX(Days+1,2)</f><v>2</v>",
        "<f>DATE(Yr,MonOpt,1)</f><v>43922</v>",
        "<f ca=\"1\">SUM(Filled)</f><v>21</v>",
        "<f ca=\"1\">ROWS(Filled)</f><v>3</v>",
        "<f ca=\"1\">MATCH(7,Filled,0)</f><v>2</v>",
        "<f ca=\"1\">INDEX(Filled,2)</f><v>7</v>",
        "<f ca=\"1\">COUNTIF(Filled,\"&gt;5\")</f><v>2</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn sheet_qualified_names_resolve_in_their_sheet() {
    // Sheet1!Yr names the sheet-level name Yr of Sheet1, and names may be
    // defined in terms of names that come later in the file.
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>2021</v></c><c r=\"B1\"><f>firstdate</f><v>0</v></c><c r=\"C1\"><f>Sheet1!firstdate</f><v>0</v></c><c r=\"E1\"><f>INDEX(calendar,2)</f><v>0</v></c><c r=\"F1\"><f>YrNext</f><v>0</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</workbook>",
        "<definedNames><definedName name=\"calendar\" localSheetId=\"0\">days+Sheet1!firstdate</definedName><definedName name=\"days\">{0,1,2,3,4,5,6}</definedName><definedName name=\"firstdate\" localSheetId=\"0\">DATE(Sheet1!Yr,1,1)</definedName><definedName name=\"Yr\" localSheetId=\"0\">Sheet1!$A$1</definedName><definedName name=\"YrNext\">Sheet1!Yr+1</definedName></definedNames></workbook>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f>firstdate</f><v>44197</v>",
        "<f>Sheet1!firstdate</f><v>44197</v>",
        "<f>INDEX(calendar,2)</f><v>44198</v>",
        "<f>YrNext</f><v>2022</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn range_names_are_references() {
    // INDEX, OFFSET and ROWS take a range name as the range it names.
    let mut p = parts(
        "<row r=\"2\"><c r=\"B2\"><v>2</v></c></row><row r=\"6\"><c r=\"B6\" cm=\"1\"><f t=\"array\" ref=\"B6\">INDEX(Years,B2,1)</f><v>0</v></c><c r=\"C6\"><f>INDEX(Years,B2,1)</f><v>0</v></c><c r=\"D6\"><f>OFFSET(Years,1,0,1,1)</f><v>0</v></c><c r=\"E6\"><f>ROWS(Years)</f><v>0</v></c></row><row r=\"18\"><c r=\"B18\"><v>2022</v></c></row><row r=\"19\"><c r=\"B19\"><v>2023</v></c></row><row r=\"20\"><c r=\"B20\"><v>2024</v></c></row><row r=\"21\"><c r=\"B21\"><v>1</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</workbook>",
        "<definedNames><definedName name=\"Years\">Sheet1!$B$18:$B$20</definedName></definedNames></workbook>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "INDEX(Years,B2,1)</f><v>2023</v></c><c r=\"C6\"><f>INDEX(Years,B2,1)</f><v>2023</v>",
        "<f ca=\"1\">OFFSET(Years,1,0,1,1)</f><v>2023</v>",
        "<f>ROWS(Years)</f><v>3</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn formulas_without_the_array_flag_take_the_implicit_intersection() {
    // A formula stored without t="array" is a legacy formula: an array result
    // shows its top-left value and a range result the cell in the formula's
    // row or column. A legacy array formula fills exactly its extent.
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>10</v></c><c r=\"C1\"><f>{1,2,3}</f><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>20</v></c><c r=\"B2\"><f>A1:A3</f><v>0</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>30</v></c></row>\
         <row r=\"5\"><c r=\"B5\"><f t=\"array\" ref=\"B5:C6\">{1,2,3;4,5,6;7,8,9}</f><v>0</v></c><c r=\"C5\"><v>0</v></c><c r=\"E5\"><f t=\"array\" ref=\"E5:E7\">{1;2}</f><v>0</v></c></row>\
         <row r=\"6\"><c r=\"B6\"><v>0</v></c><c r=\"C6\"><v>0</v></c><c r=\"E6\"><v>0</v></c></row>\
         <row r=\"7\"><c r=\"E7\"><v>0</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f>{1,2,3}</f><v>1</v>",
        "<f>A1:A3</f><v>20</v>",
        "<f t=\"array\" ref=\"B5:C6\">{1,2,3;4,5,6;7,8,9}</f><v>1</v></c><c r=\"C5\"><v>2</v>",
        "<c r=\"B6\"><v>4</v></c><c r=\"C6\"><v>5</v>",
        "<c r=\"E6\"><v>2</v>",
        "<c r=\"E7\" t=\"e\"><v>#N/A</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn formulas_without_the_array_flag_intersect_inside_the_formula() {
    // A legacy formula intersects a range or a name for one in a single-value
    // position with its own row (IF's test, an operator operand), ROW gives
    // its first row, and SUMPRODUCT still evaluates its argument as an array;
    // the same text entered as an array does not intersect.
    let mut p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>10</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>0</v></c><c r=\"B2\"><v>20</v></c><c r=\"C2\"><f>MAX(IF(A1:A3=0,B1:B3))</f><v>0</v></c><c r=\"D2\"><f t=\"array\" ref=\"D2\">MAX(IF(A1:A3=0,B1:B3))</f><v>0</v></c><c r=\"E2\" t=\"str\"><f>IF(Flags=0,\"zero\",\"one\")</f><v></v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>1</v></c><c r=\"B3\"><v>30</v></c><c r=\"C3\"><f>SUM(ROW(A1:A3))</f><v>0</v></c><c r=\"D3\"><f t=\"array\" ref=\"D3\">SUM(ROW(A1:A3))</f><v>0</v></c></row>\
         <row r=\"5\"><c r=\"C5\"><f>SUMPRODUCT((A1:A3=1)*B1:B3)</f><v>0</v></c><c r=\"D5\"><f>SUM((A1:A3=1)*B1:B3)</f><v>0</v></c></row>",
    );
    let wb = p.get_mut("xl/workbook.xml").unwrap();
    *wb = wb.replace(
        "</workbook>",
        "<definedNames><definedName name=\"Flags\">Sheet1!$A$1:$A$3</definedName></definedNames></workbook>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f>MAX(IF(A1:A3=0,B1:B3))</f><v>30</v>",
        "<f>IF(Flags=0,\"zero\",\"one\")</f><v>zero</v>",
        "<f t=\"array\" ref=\"D2\">MAX(IF(A1:A3=0,B1:B3))</f><v>20</v>",
        "<f>SUM(ROW(A1:A3))</f><v>1</v>",
        "<f t=\"array\" ref=\"D3\">SUM(ROW(A1:A3))</f><v>6</v>",
        "<f>SUMPRODUCT((A1:A3=1)*B1:B3)</f><v>40</v>",
        "<c r=\"D5\" t=\"e\"><f>SUM((A1:A3=1)*B1:B3)</f><v>#VALUE!</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn formulas_without_the_array_flag_lift_reference_parameters_over_arrays_of_references() {
    // Inside SUMPRODUCT OFFSET with ROW(B1:B3)-1 is an array of references
    // that SUBTOTAL evaluates once per reference, while the operand next to
    // SUMPRODUCT intersects A1:A3 with row 3; outside an array argument ROW
    // gives its first row, and OFFSET's rows intersects a range.
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>10</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>0</v></c><c r=\"B2\"><v>20</v></c><c r=\"C2\"><f>SUBTOTAL(9,OFFSET(B1,A1:A3,0))</f><v>0</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>1</v></c><c r=\"B3\"><v>30</v></c><c r=\"C3\"><f>SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0))*{1;10;100})+A1:A3*5</f><v>0</v></c><c r=\"D3\"><f>SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>0</v></c><c r=\"E3\"><f t=\"array\" ref=\"E3\">SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>0</v></c></row>\
         <row r=\"5\"><c r=\"C5\"><f>SUMPRODUCT(SUBTOTAL(3,OFFSET(A1,ROW(A2:A4)-1,0)))</f><v>0</v></c><c r=\"D5\"><f>SUMPRODUCT(N(OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>0</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f ca=\"1\">SUBTOTAL(9,OFFSET(B1,A1:A3,0))</f><v>10</v>",
        "<f ca=\"1\">SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0))*{1;10;100})+A1:A3*5</f><v>3215</v>",
        "<f ca=\"1\">SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>10</v>",
        "<f t=\"array\" ref=\"E3\" ca=\"1\">SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>60</v>",
        "<f ca=\"1\">SUMPRODUCT(SUBTOTAL(3,OFFSET(A1,ROW(A2:A4)-1,0)))</f><v>2</v>",
        "<f ca=\"1\">SUMPRODUCT(N(OFFSET(B1,ROW(B1:B3)-1,0)))</f><v>60</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn formulas_reading_array_members_see_the_array_result() {
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><f>C3*10</f><v>0</v></c><c r=\"B1\"><f>D3+1</f><v>0</v></c></row>\
         <row r=\"3\"><c r=\"B3\"><f t=\"array\" ref=\"B3:D3\">{1,2,3}+A5</f><v>1</v></c><c r=\"C3\"><v>2</v></c><c r=\"D3\"><v>3</v></c></row>\
         <row r=\"5\"><c r=\"A5\"><v>100</v></c><c r=\"B5\"><f>C7*10</f><v>0</v></c></row>\
         <row r=\"7\"><c r=\"B7\" cm=\"1\"><f t=\"array\" ref=\"B7:D7\">{1,2,3}+A5</f><v>1</v></c><c r=\"C7\"><v>2</v></c><c r=\"D7\"><v>3</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "<f>C3*10</f><v>1020</v>",
        "<f>D3+1</f><v>104</v>",
        "<f>C7*10</f><v>1020</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn references_that_only_look_circular_are_calculated() {
    // Each K cell looks up an earlier K cell through a range that includes
    // itself; nothing reads its own value, so Excel calculates them.
    let rows: String = [(3, "a"), (4, "b"), (5, "a"), (6, "c")]
        .iter()
        .map(|(r, key)| {
            format!(
                "<row r=\"{r}\"><c r=\"J{r}\" t=\"inlineStr\"><is><t>{key}</t></is></c><c r=\"K{r}\"><f>IF(COUNTIF($J$3:J{r},J{r})=1,MAX($K$2:K{p})+1,INDEX($K$3:K{r},MATCH(J{r},$J$3:J{r},0)))</f><v>0</v></c></row>",
                p = r - 1
            )
        })
        .collect();
    let out = recalculate_xlsx_bytes(&pack(&parts(&rows)), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for (cell, value) in [("K3", 1), ("K4", 2), ("K5", 1), ("K6", 3)] {
        let at = sheet.find(&format!("r=\"{cell}\"")).unwrap();
        assert!(
            sheet[at..].contains(&format!("</f><v>{value}</v>"))
                && sheet[at..].find(&format!("</f><v>{value}</v>")) < sheet[at..].find("</c>"),
            "{cell}={value}: {sheet}"
        );
    }
}
#[test]
fn whole_columns_span_the_grid() {
    // A:A and D:D line up row by row, and INDEX reaches rows past the data.
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"D1\"><v>9</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"D2\"><v>9</v></c></row>\
         <row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>x</t></is></c></row>\
         <row r=\"20\"><c r=\"P20\"><f>SUMPRODUCT((A:A&gt;0)*(D:D=9))</f><v>0</v></c><c r=\"Q20\"><f>SUMPRODUCT(A:A,D:D)</f><v>0</v></c><c r=\"R20\"><f>INDEX($A:$A,65536)&amp;\"\"</f><v>0</v></c><c r=\"S20\"><f>ROWS(INDEX($A:$C,0,2))</f><v>0</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    for expected in [
        "(D:D=9))</f><v>2</v>",
        "<f>SUMPRODUCT(A:A,D:D)</f><v>27</v>",
        "<f>INDEX($A:$A,65536)&amp;\"\"</f><v></v>",
        "<f>ROWS(INDEX($A:$C,0,2))</f><v>1048576</v>",
    ] {
        assert!(sheet.contains(expected), "{expected}: {sheet}");
    }
}
#[test]
fn arrays_larger_than_ten_thousand_cells_fill_their_extent() {
    // A legacy array over 1001 x 10 cells (more than the engine's default
    // spill cap) fills its extent like any other.
    let mut rows = String::new();
    for r in 1..=1001 {
        rows.push_str(&format!("<row r=\"{r}\">"));
        for c in ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"] {
            if (r, c) == (1, "A") {
                rows.push_str("<c r=\"A1\"><f t=\"array\" ref=\"A1:J1001\">ROW(L1:U1001)*100+COLUMN(L1:U1001)</f><v>0</v></c>");
            } else {
                rows.push_str(&format!("<c r=\"{c}{r}\"><v>0</v></c>"));
            }
        }
        rows.push_str("</row>");
    }
    let out = recalculate_xlsx_bytes(&pack(&parts(&rows)), Default::default()).unwrap();
    let sheet = member(&out.bytes, SHEET);
    assert!(sheet.contains("ref=\"A1:J1001\">ROW(L1:U1001)*100+COLUMN(L1:U1001)</f><v>112</v>"));
    assert!(
        sheet.contains("<c r=\"J1001\"><v>100121</v>"),
        "{}",
        &sheet[sheet.len() - 300..]
    );
}
#[test]
fn cell_filename_names_the_workbook_and_sheet() {
    let p = parts(
        "<row r=\"1\"><c r=\"A1\"><f>MID(CELL(\"filename\",A1),FIND(\"]\",CELL(\"filename\",A1))+1,255)</f><v>0</v></c></row>",
    );
    let out = recalculate_xlsx_bytes(&pack(&p), Default::default()).unwrap();
    assert!(member(&out.bytes, SHEET).contains("<v>Sheet1</v>"));
    let mut options = XlsxRecalculateOptions::default();
    options.eval_config.workbook_file_name = Some("Budget.xlsx".into());
    let p = parts("<row r=\"1\"><c r=\"A1\"><f>CELL(\"filename\",A1)</f><v>0</v></c></row>");
    let out = recalculate_xlsx_bytes(&pack(&p), options).unwrap();
    assert!(member(&out.bytes, SHEET).contains("<v>[Budget.xlsx]Sheet1</v>"));
}
