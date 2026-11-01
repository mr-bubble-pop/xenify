//! DOCX written the way Word writes it (hand-made XML, not our writer):
//! superscripts, character styles, caps, exact line spacing, theme fonts,
//! links to bookmarks, floating and VML pictures, text boxes, comment
//! threads and tracked changes all come in.
use xenify_wp_doc::{Align, Block, Field, VertAlign};
use xenify_wp_io::zip::ZipWriter;
use xenify_wp_io::{Format, load};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8,
    6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0,
    0xF0, 0x1F, 0, 0x05, 0, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60,
    0x82,
];

fn picture(anchor: bool) -> String {
    let graphic = r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdImg"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>"#;
    if anchor {
        format!(
            r#"<w:r><w:drawing><wp:anchor behindDoc="0" allowOverlap="1"><wp:positionH relativeFrom="margin"><wp:align>right</wp:align></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1270000" cy="635000"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="2" name="Picture 2" descr="Logo"/>{graphic}</wp:anchor></w:drawing></w:r>"#
        )
    } else {
        String::new()
    }
}

fn fixture() -> Vec<u8> {
    let body = format!(
        r#"<w:body>
<w:p><w:r><w:t>H</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="subscript"/></w:rPr><w:t>2</w:t></w:r><w:r><w:t xml:space="preserve">O and E=mc</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>2</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:rStyle w:val="Emphasis"/></w:rPr><w:t>styled</w:t></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r><w:r><w:rPr><w:caps/></w:rPr><w:t>shout</w:t></w:r></w:p>
<w:p><w:pPr><w:spacing w:line="280" w:lineRule="exact"/></w:pPr><w:r><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/></w:rPr><w:t>Exactly 14 points, in the heading font.</w:t></w:r></w:p>
<w:p><w:hyperlink w:anchor="later"><w:r><w:t>Jump</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> to the end.</w:t></w:r></w:p>
<w:p>{}</w:p>
<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><wp:extent cx="1000000" cy="500000"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:txbx><w:txbxContent><w:p><w:r><w:t>Boxed text</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>Before the box.</w:t></w:r></w:p>
<w:p><w:r><w:pict><v:shape style="width:72pt;height:36pt"><v:imagedata r:id="rIdImg" o:title=""/></v:shape></w:pict></w:r></w:p>
<w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Discuss</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r><w:r><w:t xml:space="preserve"> this </w:t></w:r><w:ins w:id="5" w:author="Ana" w:date="2026-10-10T10:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="6" w:author="Ana" w:date="2026-10-10T10:00:00Z"><w:r><w:delText>old</w:delText></w:r></w:del><w:r><w:t xml:space="preserve"> plan.</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Grid</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:tbl><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Layout</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:bookmarkStart w:id="9" w:name="later"/><w:r><w:t>The end.</w:t></w:r><w:bookmarkEnd w:id="9"/></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>
</w:body>"#,
        picture(true)
    )
    .replace("o:title=\"\"", "");
    let mut z = ZipWriter::new();
    z.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        true,
    );
    z.add("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#, true);
    z.add("word/_rels/document.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#, true);
    z.add("word/document.xml", format!(r#"<?xml version="1.0"?><w:document {W}>{body}</w:document>"#).as_bytes(), true);
    z.add("word/media/image1.png", PNG, false);
    z.add("word/styles.xml", format!(r#"<?xml version="1.0"?><w:styles {W}><w:style w:type="table" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="auto"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr></w:style><w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/><w:color w:val="C00000"/></w:rPr></w:style></w:styles>"#).as_bytes(), true);
    z.add("word/theme/theme1.xml", br#"<?xml version="1.0"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#, true);
    z.add("word/comments.xml", format!(r#"<?xml version="1.0"?><w:comments {W}><w:comment w:id="0" w:author="Sam" w:date="2026-10-09T09:00:00Z"><w:p w14:paraId="1A2B3C4D"><w:r><w:annotationRef/></w:r><w:r><w:t>Why?</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ana"><w:p w14:paraId="1A2B3C4E"><w:r><w:t>Because.</w:t></w:r></w:p></w:comment></w:comments>"#).as_bytes(), true);
    z.add("word/commentsExtended.xml", br#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="1A2B3C4D" w15:done="0"/><w15:commentEx w15:paraId="1A2B3C4E" w15:paraIdParent="1A2B3C4D" w15:done="0"/></w15:commentsEx>"#, true);
    z.finish()
}

#[test]
fn word_features_come_in() {
    let loaded = load(&fixture(), Format::Docx).unwrap();
    let d = &loaded.doc;
    let paras: Vec<_> = d.paragraphs().collect();
    let text: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
    // Superscript and subscript.
    assert_eq!(text[0], "H2O and E=mc2");
    let vert: Vec<Option<VertAlign>> = paras[0].runs.iter().map(|r| r.fmt.vert).collect();
    assert_eq!(vert, [None, Some(VertAlign::Subscript), None, Some(VertAlign::Superscript)]);
    // A character style's formatting, and caps.
    assert_eq!(text[1], "styled SHOUT");
    assert_eq!(paras[1].runs[0].fmt.italic, Some(true));
    assert_eq!(paras[1].runs[0].fmt.color.map(|c| c.to_hex()), Some("#c00000".into()));
    // Exact line spacing and the theme's heading font.
    assert_eq!(paras[2].props.line_exact, Some(14.0));
    assert_eq!(paras[2].runs[0].fmt.font.as_deref(), Some("Calibri Light"));
    // A link to a bookmark goes to its paragraph.
    let end = paras.iter().find(|p| p.text == "The end.").unwrap().id;
    assert_eq!(paras[3].runs[0].fmt.link, Some(format!("#p{}", end.0)));
    // The floating picture keeps its side; the VML picture its size.
    let images: Vec<_> =
        d.sections[0].blocks.iter().filter_map(|b| if let Block::Image(i) = b { Some(i) } else { None }).collect();
    assert_eq!(images.len(), 2, "{:#?}", d.sections[0].blocks);
    assert_eq!((images[0].align, images[0].alt.as_str(), images[0].width), (Align::Right, "Logo", 100.0));
    assert_eq!((images[1].width, images[1].height), (72.0, 36.0));
    // The text box's text follows its paragraph, boxed.
    let boxed = paras.iter().find(|p| p.text == "Boxed text").expect("text box text");
    assert!(boxed.props.borders.is_some());
    // Table lines come from the table style; a table with none has none.
    let tables: Vec<_> =
        d.sections[0].blocks.iter().filter_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).collect();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].border.map(|b| b.width), Some(0.5));
    assert!(tables[1].border.is_none());
    // The comment thread with its reply, and the tracked changes.
    assert_eq!(d.meta.comments.len(), 1);
    assert_eq!((d.meta.comments[0].text.as_str(), d.meta.comments[0].replies[0].text.as_str()), ("Why?", "Because."));
    let changes = xenify_wp_doc::review::changes(d);
    assert_eq!(changes.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["new", "old"]);
    assert!(paras.iter().all(|p| p.runs.iter().all(|r| !matches!(r.fmt.field, Some(Field::Note { .. })))));
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

#[test]
fn our_docx_keeps_these_features() {
    let doc = load(&fixture(), Format::Docx).unwrap().doc;
    let back = load(&xenify_wp_io::save(&doc, Format::Docx).unwrap(), Format::Docx).unwrap().doc;
    let shape = |d: &xenify_wp_doc::Document| {
        d.paragraphs()
            .map(|p| {
                format!(
                    "{} {:?} {:?}",
                    p.text,
                    p.props.line_exact,
                    p.runs.iter().map(|r| (r.fmt.vert, r.fmt.link.is_some())).collect::<Vec<_>>()
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&back), shape(&doc));
    // The link still reaches the right paragraph.
    let end = back.paragraphs().find(|p| p.text == "The end.").unwrap().id;
    let link = back.paragraphs().nth(3).unwrap().runs[0].fmt.link.clone();
    assert_eq!(link, Some(format!("#p{}", end.0)));
}

#[test]
fn markdown_and_igdoc_keep_these_features() {
    let doc = load(&fixture(), Format::Docx).unwrap().doc;
    let shape = |d: &xenify_wp_doc::Document| {
        let target = |l: &Option<String>| {
            l.as_deref()
                .and_then(|l| l.strip_prefix("#p"))
                .and_then(|n| n.parse().ok())
                .and_then(|n| d.para(xenify_wp_doc::NodeId(n)))
                .map(|p| p.text.clone())
        };
        d.paragraphs()
            .map(|p| {
                format!(
                    "{} {:?} {:?}",
                    p.text,
                    p.props.line_exact,
                    p.runs.iter().map(|r| (r.fmt.vert, target(&r.fmt.link))).collect::<Vec<_>>()
                )
            })
            .collect::<Vec<_>>()
    };
    for format in [Format::Markdown, Format::Igdoc] {
        let bytes = xenify_wp_io::save(&doc, format).unwrap();
        let back = load(&bytes, format).unwrap().doc;
        let (a, b) = (shape(&doc), shape(&back));
        // Markdown only anchors headings, so the link to a body paragraph is the one thing allowed to go.
        if format == Format::Markdown {
            let strip =
                |v: Vec<String>| v.into_iter().map(|s| s.replace("Some(\"The end.\")", "None")).collect::<Vec<_>>();
            assert_eq!(strip(b), strip(a), "{}", String::from_utf8_lossy(&bytes));
        } else {
            assert_eq!(b, a);
        }
    }
}

#[test]
fn rich_headers_round_trip() {
    use xenify_wp_doc::{CharFmt, HeaderPart, HfSlot, Image, Paragraph, Run};
    let mut doc = xenify_wp_doc::Document::from_plain_text("Body.");
    doc.assets.insert("media/logo.png".into(), std::sync::Arc::new(PNG.to_vec()));
    let (h, p1, img, f, p2) = (doc.alloc_id(), doc.alloc_id(), doc.alloc_id(), doc.alloc_id(), doc.alloc_id());
    let mut p = Paragraph::new(p1, "header", "Acme Ltd — page ");
    p.runs = vec![
        Run { len: 4, fmt: CharFmt { bold: Some(true), ..Default::default() } },
        Run { len: p.text.len() - 4, fmt: CharFmt::default() },
    ];
    p.text.push(Field::CHAR);
    p.runs.push(Run { len: Field::CHAR.len_utf8(), fmt: CharFmt { field: Some(Field::Page), ..Default::default() } });
    doc.sections[0].headers.push(HeaderPart {
        id: h,
        footer: false,
        slot: HfSlot::Default,
        blocks: vec![
            Block::Image(Image {
                id: img,
                src: "media/logo.png".into(),
                width: 60.0,
                height: 20.0,
                align: Align::Right,
                ..Default::default()
            }),
            Block::Paragraph(p),
        ],
    });
    doc.sections[0].page.different_first = true;
    doc.sections[0].headers.push(HeaderPart {
        id: f,
        footer: true,
        slot: HfSlot::First,
        blocks: vec![Block::Paragraph({
            let mut q = Paragraph::new(p2, "footer", "Draft — first page");
            q.runs[0].fmt.italic = Some(true);
            q
        })],
    });
    let shape = |d: &xenify_wp_doc::Document| {
        let mut v: Vec<String> = d.sections[0]
            .headers
            .iter()
            .map(|h| {
                let blocks: Vec<String> = h
                    .blocks
                    .iter()
                    .map(|b| match b {
                        Block::Paragraph(p) => format!(
                            "{} {:?}",
                            p.text,
                            p.runs.iter().map(|r| (r.fmt.bold, r.fmt.italic, r.fmt.field.clone())).collect::<Vec<_>>()
                        ),
                        Block::Image(i) => format!("image {}x{} {:?}", i.width, i.height, i.align),
                        _ => "other".into(),
                    })
                    .collect();
                format!("{} {:?} {blocks:?}", h.footer, h.slot)
            })
            .collect();
        v.sort();
        v
    };
    let want = shape(&doc);
    for format in [Format::Igdoc, Format::Docx, Format::Markdown] {
        let bytes = xenify_wp_io::save(&doc, format).unwrap();
        let back = load(&bytes, format).unwrap();
        assert_eq!(shape(&back.doc), want, "{format:?}");
        // The picture's bytes came along (Markdown refers to the file instead).
        if format != Format::Markdown {
            assert!(back.doc.assets.values().any(|a| a.as_slice() == PNG), "{format:?}");
        }
        // Body text is still just the body.
        assert_eq!(back.doc.sections[0].blocks.len(), 1, "{format:?}");
    }
}

#[test]
fn a_plain_one_line_header_stays_a_simple_line() {
    use xenify_wp_doc::{HeaderPart, HfSlot, Paragraph};
    let mut doc = xenify_wp_doc::Document::from_plain_text("Body.");
    let (h, p) = (doc.alloc_id(), doc.alloc_id());
    doc.sections[0].headers.push(HeaderPart {
        id: h,
        footer: false,
        slot: HfSlot::Default,
        blocks: vec![Block::Paragraph(Paragraph::new(p, "header", "Quarterly report"))],
    });
    let back = load(&xenify_wp_io::save(&doc, Format::Docx).unwrap(), Format::Docx).unwrap().doc;
    assert!(back.sections[0].headers.is_empty());
    assert_eq!(back.sections[0].page.header.center, "Quarterly report");
}
