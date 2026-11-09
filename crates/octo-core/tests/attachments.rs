use octo_core::CoreError;
use octo_core::attachments::{MAX_TEXT_BYTES, build_prompt, decode_text};

#[test]
fn prompt_without_attachments_is_unchanged() {
    assert_eq!(build_prompt("hi", &[]), "hi");
}

#[test]
fn inlines_as_fenced_blocks_with_path_header() {
    let p = build_prompt("explain", &[("notes/a.txt".into(), "line1\nline2".into())]);
    assert_eq!(p, "explain\n\nnotes/a.txt:\n```\nline1\nline2\n```");
}

#[test]
fn multiple_attachments_keep_order() {
    let p = build_prompt("q", &[("a".into(), "1".into()), ("b".into(), "2".into())]);
    assert!(p.find("a:\n").unwrap() < p.find("b:\n").unwrap());
}

#[test]
fn fence_grows_when_content_has_backticks() {
    let p = build_prompt("q", &[("a.md".into(), "x\n```rust\ny\n```\n".into())]);
    assert!(p.contains("\n````\nx\n```rust"), "{p}");
    assert!(p.ends_with("\n````"), "{p}");
}

#[test]
fn decode_accepts_utf8_up_to_limit() {
    assert_eq!(decode_text("a.txt", "привет".as_bytes()).unwrap(), "привет");
    assert!(decode_text("a.txt", &vec![b'a'; MAX_TEXT_BYTES]).is_ok());
    assert_eq!(MAX_TEXT_BYTES, 200 * 1024);
}

#[test]
fn decode_rejects_big_binary_and_invalid_utf8() {
    for data in [
        vec![b'a'; MAX_TEXT_BYTES + 1],
        vec![0xff, 0xfe, 0x00],
        b"a\0b".to_vec(),
    ] {
        assert!(matches!(
            decode_text("x.bin", &data),
            Err(CoreError::UnsupportedMedia(_))
        ));
    }
}

// ---- stage 6: images and PDF ----
use octo_core::attachments::{
    Kind, Loaded, MAX_IMAGE_BYTES, load, pdf_text, sniff, with_image_paths,
};

pub fn png() -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1]);
    v
}

/// Minimal one-page PDF; `lines` are drawn as text when `Some`, otherwise the page is empty
/// (like a scan without a text layer). Offsets in the xref table are exact.
pub fn pdf(lines: Option<&[&str]>) -> Vec<u8> {
    let mut content = String::new();
    if let Some(lines) = lines {
        content.push_str("BT /F1 10 Tf 12 TL 40 800 Td\n");
        for l in lines {
            content.push_str(&format!("({l}) Tj T*\n"));
        }
        content.push_str("ET");
    }
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn sniff_recognises_content_not_extension() {
    assert_eq!(sniff(&png()), Kind::Image("image/png"));
    assert_eq!(
        sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10]),
        Kind::Image("image/jpeg")
    );
    assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Kind::Image("image/webp"));
    assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), Kind::Image("image/gif"));
    assert_eq!(sniff(b"GIF87a\x01\0\x01\0"), Kind::Image("image/gif"));
    assert_eq!(sniff(&pdf(Some(&["x"]))), Kind::Pdf);
    assert_eq!(sniff("привет\n".as_bytes()), Kind::Text);
    assert_eq!(sniff(b""), Kind::Text);
    assert_eq!(sniff(&[0xff, 0x00, 0x01, 0x02]), Kind::Other);
    // RIFF that is not WebP, truncated magic
    assert_eq!(sniff(b"RIFF\x24\0\0\0WAVEfmt "), Kind::Other);
    assert_eq!(sniff(b"\x89PNG"), Kind::Other);
}

#[test]
fn load_dispatches_by_kind() {
    assert_eq!(
        load("a.png", &png()).unwrap(),
        Loaded::Image {
            mime: "image/png",
            data: png()
        }
    );
    // an image with a .txt name is still an image
    assert!(matches!(
        load("a.txt", &png()).unwrap(),
        Loaded::Image { .. }
    ));
    assert_eq!(
        load("a.txt", b"hello").unwrap(),
        Loaded::Text("hello".into())
    );
    // a text file named .pdf is text
    assert_eq!(
        load("a.pdf", b"hello").unwrap(),
        Loaded::Text("hello".into())
    );
    let Loaded::Text(t) = load("lab.pdf", &pdf(Some(&["Hemoglobin 14.2"]))).unwrap() else {
        panic!("pdf must load as text");
    };
    assert!(t.contains("Hemoglobin 14.2"), "{t}");
    assert!(matches!(
        load("x.bin", &[0xff, 0, 1]),
        Err(CoreError::UnsupportedMedia(_))
    ));
}

#[test]
fn image_size_cap() {
    let mut ok = png();
    ok.resize(MAX_IMAGE_BYTES, 0);
    assert!(matches!(load("a.png", &ok).unwrap(), Loaded::Image { .. }));
    ok.push(0);
    assert!(matches!(
        load("a.png", &ok),
        Err(CoreError::UnsupportedMedia(_))
    ));
    assert_eq!(MAX_IMAGE_BYTES, 10 * 1024 * 1024);
}

#[test]
fn pdf_text_layer_is_extracted_in_order() {
    let t = pdf_text(
        "a.pdf",
        &pdf(Some(&["Glucose 5.1 mmol/L", "Cholesterol 4.2"])),
    )
    .unwrap();
    assert!(t.contains("Glucose 5.1 mmol/L"), "{t}");
    assert!(
        t.find("Glucose").unwrap() < t.find("Cholesterol").unwrap(),
        "{t}"
    );
}

#[test]
fn pdf_without_text_layer_is_unsupported() {
    assert!(matches!(
        pdf_text("scan.pdf", &pdf(None)),
        Err(CoreError::UnsupportedMedia(_))
    ));
    assert!(matches!(
        load("scan.pdf", &pdf(None)),
        Err(CoreError::UnsupportedMedia(_))
    ));
    // garbage after the header
    assert!(matches!(
        pdf_text("x.pdf", b"%PDF-1.4\nnot a pdf"),
        Err(CoreError::UnsupportedMedia(_))
    ));
}

#[test]
fn pdf_text_is_capped_with_a_marker_line() {
    let line = "Hemoglobin 14.2 g/dL reference range 13.0 - 17.0";
    let lines: Vec<&str> = std::iter::repeat_n(line, 8000).collect();
    let t = pdf_text("big.pdf", &pdf(Some(&lines))).unwrap();
    assert!(t.len() <= MAX_TEXT_BYTES, "{}", t.len());
    assert!(
        t.lines().last().unwrap().contains("truncated"),
        "{}",
        &t[t.len() - 80..]
    );
    assert!(t.starts_with("Hemoglobin"));
}

#[test]
fn cli_prompt_gets_one_line_per_image() {
    assert_eq!(with_image_paths("look", &[]), "look");
    assert_eq!(
        with_image_paths(
            "look",
            &[
                "/data/public/a.png".into(),
                "/data/public/b dir/c.jpg".into()
            ]
        ),
        "look\n\nAttached image: /data/public/a.png\nAttached image: /data/public/b dir/c.jpg"
    );
}

/// PDF with a Type0/Identity-H font and a ToUnicode CMap: glyph ids 1..=n map to `text`.
fn cyrillic_pdf(text: &str) -> Vec<u8> {
    let chars: Vec<char> = text.chars().collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", chars.len()));
    for (i, c) in chars.iter().enumerate() {
        cmap.push_str(&format!("<{:04X}> <{:04X}>\n", i + 1, *c as u32));
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end");
    let hex: String = (1..=chars.len()).map(|i| format!("{i:04X}")).collect();
    let content = format!("BT /F1 12 Tf 40 800 Td <{hex}> Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
        "<< /Type /Font /Subtype /Type0 /BaseFont /Arial /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>".to_string(),
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Arial /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 /FontDescriptor 8 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{cmap}\nendstream", cmap.len() + 1),
        "<< /Type /FontDescriptor /FontName /Arial /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn pdf_cid_font_with_to_unicode_gives_cyrillic() {
    let t = pdf_text("lab.pdf", &cyrillic_pdf("Гемоглобин 140 г/л")).unwrap();
    assert!(t.contains("Гемоглобин 140 г/л"), "{t}");
}

#[test]
fn hostile_pdfs_never_panic() {
    let good = pdf(Some(&["x"]));
    for n in [9, 40, good.len() / 2, good.len() - 10] {
        let _ = pdf_text("x.pdf", &good[..n]);
    }
    let mut flipped = good.clone();
    for i in (20..flipped.len()).step_by(7) {
        flipped[i] ^= 0x55;
    }
    let _ = pdf_text("x.pdf", &flipped);
    let mut big = b"%PDF-1.4\n".to_vec();
    big.resize(octo_core::attachments::MAX_PDF_BYTES + 1, b' ');
    assert!(matches!(
        pdf_text("x.pdf", &big),
        Err(CoreError::UnsupportedMedia(_))
    ));
}

fn pdf_pages(n: usize) -> Vec<u8> {
    let kids: String = (0..n).map(|i| format!("{} 0 R ", 4 + i)).collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 4 + n + i));
    }
    for _ in 0..n {
        let c = "BT /F1 10 Tf 40 800 Td (page) Tj ET";
        objs.push(format!(
            "<< /Length {} >>\nstream\n{c}\nendstream",
            c.len() + 1
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn pdf_page_count_is_capped_at_300() {
    assert!(pdf_text("a.pdf", &pdf_pages(300)).is_ok());
    assert!(matches!(
        pdf_text("a.pdf", &pdf_pages(301)),
        Err(CoreError::UnsupportedMedia(_))
    ));
}
