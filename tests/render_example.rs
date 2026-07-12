//! Integration test that doubles as the document generator: it assembles a
//! multi-page field guide to the mantis shrimp with the imperative
//! [`textris_pdf::build`] API, renders it end-to-end, and writes the resulting PDF
//! to disk.
//!
//! Run just this test to (re)generate the PDF:
//!
//! ```text
//! cargo test --test render_example -- renders_example_to_pdf_on_disk
//! ```
//!
//! The construction below drives the repeated content (species rows, the field
//! checklist) from data with ordinary loops and conditionals, the payoff of
//! building the document in code. It exercises the full block vocabulary:
//! auto-numbered and referenced headings, paragraphs with hard line breaks,
//! tables (including fill-in and spacer cells), task lists, a boxed callout,
//! both numbered and lettered ordered lists, a vertical spacer, and an
//! explicit page break.

use std::path::Path;

use textris_pdf::{
    build::{Textris, bold, cell, fill_in, italic, mono, muted, spacer, text},
    fonts::Fonts,
    model::{ListMarker, SectionContent},
    theme::em,
};

#[test]
fn renders_example_to_pdf_on_disk() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fonts = load_fonts().expect("fonts should load");

    let pdf = sample()
        .render(&fonts)
        .expect("document should render as valid PDF/A-2b");

    // It should be a real, non-trivial PDF.
    assert!(pdf.starts_with(b"%PDF-"), "output is not a PDF");
    assert!(pdf.len() > 4000, "PDF looks too small: {} bytes", pdf.len());

    // Write it out so it can be inspected.
    let out = root.join("tests/mantis-shrimp-example.pdf");
    std::fs::write(&out, &pdf).expect("should write PDF to disk");
    assert!(out.exists());
}

/// Load the fonts the example uses, Newsreader (roman + italic variable fonts)
/// and Fira Code, from the crate's `tests/fonts` directory. This is the
/// one place the concrete typefaces are named; the library itself is font-agnostic.
fn load_fonts() -> std::io::Result<Fonts> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fonts");
    Fonts::from_variable_files(
        dir.join("Newsreader/Newsreader-Variable.ttf"),
        dir.join("Newsreader/Newsreader-Italic-Variable.ttf"),
        dir.join("Fira_Code/FiraCode-Variable.ttf"),
    )
}

/// A notable mantis shrimp species.
struct Species {
    common_name: &'static str,
    scientific_name: &'static str,
    strike_type: &'static str,
    max_length: &'static str,
}

/// A single physical measurement to tabulate.
struct Measurement {
    attribute: &'static str,
    value: &'static str,
    unit: &'static str,
    notes: &'static str,
}

/// An item on the field checklist, and whether it is required (checked).
struct ChecklistItem {
    required: bool,
    description: &'static str,
}

/// Assemble the full mantis shrimp field guide.
fn sample() -> Textris {
    // --- Data -------------------------------------------------------------
    // A handful of the ~450 described stomatopod species.
    let species = [
        Species {
            common_name: "Peacock mantis shrimp",
            scientific_name: "Odontodactylus scyllarus",
            strike_type: "smasher",
            max_length: "18 cm",
        },
        Species {
            common_name: "Zebra mantis shrimp",
            scientific_name: "Lysiosquillina maculata",
            strike_type: "spearer",
            max_length: "40 cm",
        },
        Species {
            common_name: "Purple-spot mantis shrimp",
            scientific_name: "Gonodactylus smithii",
            strike_type: "smasher",
            max_length: "10 cm",
        },
        Species {
            common_name: "Caribbean rock mantis shrimp",
            scientific_name: "Neogonodactylus oerstedii",
            strike_type: "smasher",
            max_length: "7 cm",
        },
        Species {
            common_name: "Spottail mantis shrimp",
            scientific_name: "Squilla mantis",
            strike_type: "spearer",
            max_length: "20 cm",
        },
        Species {
            common_name: "Giant mantis shrimp",
            scientific_name: "Hemisquilla californiensis",
            strike_type: "smasher",
            max_length: "30 cm",
        },
        Species {
            common_name: "Ciliated false squilla",
            scientific_name: "Pseudosquilla ciliata",
            strike_type: "spearer",
            max_length: "10 cm",
        },
    ];

    // Figures characterising the smashers' club strike and the animal's vision.
    let measurements = [
        Measurement {
            attribute: "Peak strike speed",
            value: "23",
            unit: "m/s",
            notes: "club tip, peacock mantis shrimp",
        },
        Measurement {
            attribute: "Strike acceleration",
            value: "10400",
            unit: "g",
            notes: "on the order of a .22 bullet",
        },
        Measurement {
            attribute: "Peak strike force",
            value: "1500",
            unit: "N",
            notes: "enough to crack aquarium glass",
        },
        Measurement {
            attribute: "Strike duration",
            value: "2.7",
            unit: "ms",
            notes: "followed by a cavitation bubble",
        },
        Measurement {
            attribute: "Photoreceptor types",
            value: "16",
            unit: "-",
            notes: "against three in the human eye",
        },
    ];

    let regions = "the Indo-Pacific from East Africa to Hawaii, the Red Sea, the \
        Mediterranean and eastern Atlantic, and the tropical western Atlantic from \
        Florida to Brazil";

    let checklist = [
        ChecklistItem {
            required: true,
            description: "Photograph the raptorial appendages so the strike type (smashing club or spearing barb) can be determined later.",
        },
        ChecklistItem {
            required: true,
            description: "Record the burrow entrance and the surrounding substrate (sand, rubble or coral).",
        },
        ChecklistItem {
            required: false,
            description: "Note the water temperature and the depth at which the animal was observed.",
        },
    ];

    // --- Document ---------------------------------------------------------
    let mut doc = Textris::new();

    doc.header_right("Stomatopoda - Field Guide");

    doc.h2("Marine species profile");
    doc.h1("The Mantis Shrimp");
    doc.paragraph(
        "Mantis shrimp are marine crustaceans of the order Stomatopoda: burrowing \
         ambush predators famous for the fastest limb strike in the animal kingdom \
         and for one of the most elaborate visual systems ever described.",
    );

    // A highlighted warning, drawn as a boxed callout.
    doc.boxed(|b| {
        b.paragraph(bold("Handle with care."));
        b.paragraph(
            "A large smasher can split a fingernail (hence the common name \
             \"thumb splitter\") and has been known to crack the glass of an \
             aquarium. Never pick one up by hand.",
        );
    });

    // A bullet list previewing the guide's contents. The section numbers are
    // forward references, resolved when the document is built.
    doc.h3("About this guide");
    doc.paragraph("This profile covers:");
    doc.bullet_list([
        text("how the group is classified and where it lives (sections ")
            .section_ref("classification")
            .normal(" and ")
            .section_ref("habitat")
            .normal(");"),
        text("the mechanics of the predatory strike and the animal's vision (sections ")
            .section_ref("strike")
            .normal(" and ")
            .section_ref("vision")
            .normal(");"),
        text("notable species and how to record a field observation (section ")
            .section_ref("record")
            .normal(" onwards)."),
    ]);

    doc.h3_numbered("Classification").anchor("classification");
    doc.paragraph(
        text("The mantis shrimp belong to the order ")
            .bold("Stomatopoda")
            .normal(" within the class ")
            .italic("Malacostraca")
            .normal(" of the crustaceans. Around ")
            .bold("450 species")
            .normal(
                " have been described, none of them a true shrimp. They \
                     diverged from other malacostracans over 340 million years \
                     ago, and the lineage's distinctive raptorial limbs are \
                     already recognisable in the fossil record.",
            ),
    );

    doc.h3_numbered("Anatomy");
    doc.paragraph(
        "The body is divided into a shielded head and thorax and a broad, \
         muscular tail. The features most worth knowing in the field are:",
    );
    doc.bullet_list([
        "a pair of stalked, independently mobile compound eyes;",
        "the folded raptorial appendages: a club in smashers, a barbed spear in spearers;",
        "the telson, the armoured tail plate used to block the burrow entrance.",
    ]);

    doc.h3_numbered("The predatory strike").anchor("strike");
    doc.paragraph(
        "Stomatopods fall into two camps. Smashers wield a folded, club-like \
         appendage that they release like a spring; spearers deploy a barbed \
         limb to impale soft-bodied prey.",
    );
    doc.paragraph(
        text(
            "A smasher's club accelerates so violently that it boils the water in \
              front of it, collapsing into a ",
        )
        .italic("cavitation bubble")
        .normal(
            " whose implosion delivers a second blow even if the first \
                     misses. The strike is among the fastest movements known in \
                     any animal, and it is aimed by the visual system described \
                     in section ",
        )
        // A forward reference: the vision section is added below.
        .section_ref("vision")
        .normal("."),
    );
    doc.paragraph(
        "The energy comes not from muscle alone but from a saddle-shaped spring in \
         the limb that is compressed and then latched, storing energy that is \
         released all at once, the same principle as a crossbow. The club itself \
         is a layered composite that resists the shattering forces it generates, \
         and has inspired the design of impact-resistant materials.",
    );

    doc.h3_numbered("Vision").anchor("vision");
    doc.paragraph(
        text(
            "Each eye moves independently and carries three regions that view the \
              same point, giving the animal ",
        )
        .bold("trinocular depth perception")
        .normal(" in a single eye."),
    );
    doc.paragraph(
        "Where humans have three colour receptors, a mantis shrimp may have as many \
         as sixteen, and can also detect both linear and circular polarised light, \
         a capability found in no other animal.",
    );
    // A numbered subsection: nested under the vision section as "4.1.".
    doc.h4_numbered("Colour discrimination");
    doc.paragraph(
        "Curiously, all those receptors do not make it a better judge of subtle \
         colour differences than we are. Current thinking is that the eye reads \
         colour more like a set of sensors than a mixing palette, trading fine \
         discrimination for a very fast, low-effort readout, useful for an \
         animal that must decide in milliseconds whether to strike.",
    );

    doc.h3_numbered("Diet and hunting");
    doc.paragraph(
        text("Smashers specialise in ")
            .bold("hard-shelled prey")
            .normal(
                " (crabs, snails and molluscs), which they batter open with \
                     repeated blows. Spearers ambush soft, fast prey such as fish \
                     and shrimp, flicking the barbed limb out faster than the eye \
                     can follow.",
            ),
    );

    doc.h3_numbered("Behaviour and life history");
    doc.paragraph(
        "Most mantis shrimp are solitary and fiercely territorial, defending a \
         burrow or rock crevice. Some species, however, are monogamous and may \
         share a burrow with the same partner for up to twenty years, with the \
         pair coordinating to defend and maintain their home.",
    );
    doc.paragraph(
        text(
            "They are unexpectedly communicative animals. Rivals settle disputes \
              with a ritualised ",
        )
        .italic("meral spread")
        .normal(
            " in which they flare their coloured limbs to warn off an \
                     opponent before any blow is struck. Several species also bear \
                     patches that ",
        )
        .bold("fluoresce")
        .normal(
            " under the blue light of deeper water, thought to signal to \
                     others of their kind.",
        ),
    );
    doc.paragraph(
        "Their reputation among people is mixed. Fishermen know them as \"prawn \
         killers\" for the catch they raid, and divers as \"thumb splitters\" for \
         the wounds a startled smasher can inflict. A few of the larger, more \
         colourful species are prized by aquarists and kept, warily, in tanks of \
         their own.",
    );

    doc.h3_numbered("Habitat and distribution")
        .anchor("habitat");
    doc.paragraph(
        text("Mantis shrimp are found in ").bold("shallow tropical and subtropical seas:"),
    );
    doc.paragraph(italic(regions));
    doc.paragraph(
        "Most species live from the intertidal zone down to a few tens of metres, \
         though some range considerably deeper. Nearly all are tied to a shelter: \
         spearers tend to dig vertical burrows in sand or mud, while smashers \
         occupy cavities in coral rubble and rock, enlarging and defending them \
         over a lifetime.",
    );

    // Section 8: notable species. Rows come from the data, numbered as we go,
    // the kind of data-driven table that motivates building in code.
    doc.h3_numbered("Notable species");
    doc.table_with(|t| {
        t.headers(["", "common name", "species", "strike", "max. length"]);
        for (i, s) in species.iter().enumerate() {
            t.row([
                text((i + 1).to_string()),
                text(s.common_name),
                italic(s.scientific_name),
                text(s.strike_type),
                mono(s.max_length),
            ]);
        }
    });

    // Section 9: measurements, a second data table, this one keyed on figures.
    doc.h3_numbered("Selected measurements");
    doc.paragraph("Representative figures for the smashers' strike and for vision:");
    doc.table_with(|t| {
        t.headers(["", "attribute", "value", "unit", "notes"]);
        for (i, m) in measurements.iter().enumerate() {
            t.row([
                text((i + 1).to_string()),
                text(m.attribute),
                mono(m.value),
                text(m.unit),
                text(m.notes),
            ]);
        }
    });

    // Section 10: field checklist, where checkbox state comes straight from the data.
    doc.h3_numbered("Field checklist").anchor("record");
    doc.paragraph("When you observe an animal in the field, try to record the following:");
    doc.task_list(checklist.iter().map(|c| (c.required, c.description)));

    // Some extra air before the next paragraph: a spacer replaces the normal
    // inter-block gap, so this is the exact distance to the checklist above.
    doc.spacer(em(2.0));

    // A lettered ordered list of what a usable record must contain.
    doc.paragraph("A complete identification record includes:");
    doc.ordered_list_with(
        ListMarker::LowerAlpha,
        [
            "a dorsal photograph showing the full body;",
            "an estimate of the total body length;",
            "the coordinates and depth of the sighting.",
        ],
    );

    // The observation record is a form to print and fill in, so it starts on
    // its own page.
    doc.page_break();

    // Section 11: a label table mixing prefilled cells, fill-in lines and a
    // tall spacer cell for free-form notes.
    doc.h3_numbered("Observation record");
    doc.label_table([
        [cell("Observer"), cell("Costa, R.")],
        [cell("Location"), cell("Lembeh Strait, Indonesia")],
        [cell("Date"), fill_in()],
        [cell("Depth"), fill_in()],
        [cell("Field notes"), spacer(em(6.0))],
        [cell("Signature"), fill_in()],
    ]);

    // Hard line breaks ('\n' or Text::line_break) keep an address inside one
    // paragraph.
    doc.paragraph(
        text("Return completed records to:")
            .line_break()
            .normal("Stomatopod Survey, Marine Field Station\nLembeh Strait, North Sulawesi"),
    );

    doc.footer_left(muted("Revision: ").mono("3"));
    doc.footer_right(SectionContent::page_counter(|page, total| {
        text(format!("Page {page} of {total}"))
    }));

    doc
}
