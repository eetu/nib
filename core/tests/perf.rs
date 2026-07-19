//! Large-document performance profile — the last finalization gate. The core's invariants are meant
//! to be **linear** in node count (parse, project-to-paths, reconcile+serialize, apply-op), but were
//! unmeasured at scale. This generates docs of growing size, times each pipeline stage, prints a
//! profile (run with `-- --nocapture`), and asserts near-linear scaling so a future O(n²) regression
//! trips CI instead of shipping. Run in release for representative numbers:
//!   cargo test -p nib-core --release --test perf -- --nocapture
use nib_core::model::document::{parse_svg, serialize_canonical};
use nib_core::ops::{Op, apply};
use std::time::{Duration, Instant};

/// An SVG with `n` small cubic paths laid on a grid — representative of a busy real document.
fn make_svg(n: usize) -> String {
    let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000">"#);
    for i in 0..n {
        let x = (i % 100) as f64 * 10.0;
        let y = (i / 100) as f64 * 10.0;
        let col = (i.wrapping_mul(2654435761)) & 0xff_ff_ff;
        s.push_str(&format!(
            "<path d=\"M {x} {y} C {a} {b}, {c} {d}, {e} {f} L {g} {h} Z\" fill=\"#{col:06x}\"/>",
            a = x + 2.0,
            b = y + 3.0,
            c = x + 5.0,
            d = y + 1.0,
            e = x + 8.0,
            f = y + 8.0,
            g = x + 1.0,
            h = y + 7.0,
        ));
    }
    s.push_str("</svg>");
    s
}

struct Row {
    n: usize,
    parse: Duration,
    project: Duration,
    serialize: Duration,
    apply_op: Duration,
}

fn measure(n: usize) -> Row {
    let svg = make_svg(n);
    // Take the MIN across a few passes per stage: minimum wall-time is the least-noisy estimate of
    // true compute cost (scheduling/GC/allocator only ever add time), so the linearity gate can't
    // flake on a busy CI runner.
    let (mut parse, mut project, mut serialize, mut apply_op) =
        (Duration::MAX, Duration::MAX, Duration::MAX, Duration::MAX);
    for _ in 0..3 {
        let t = Instant::now();
        let mut doc = parse_svg(&svg).expect("parse");
        parse = parse.min(t.elapsed());

        let t = Instant::now();
        let paths = doc.tree.as_ref().unwrap().project_paths();
        project = project.min(t.elapsed());
        doc.paths = paths;

        let t = Instant::now();
        let out = serialize_canonical(&doc, doc.tree.as_ref().unwrap(), 3);
        serialize = serialize.min(t.elapsed());
        assert!(out.len() > n, "serialized something plausible");

        // A representative edit deep in the doc: rotate the middle path about its centre.
        let t = Instant::now();
        assert!(apply(
            &mut doc,
            &Op::RotatePath {
                path: n / 2,
                degrees: 15.0,
                cx: None,
                cy: None,
            }
        ));
        apply_op = apply_op.min(t.elapsed());
    }

    Row {
        n,
        parse,
        project,
        serialize,
        apply_op,
    }
}

/// Ratio of a stage's time between two rows, normalized by the size ratio: ~1.0 = linear, ~2.0 =
/// quadratic. Guards against a stage silently going superlinear.
fn per_node_growth(small: Duration, big: Duration, size_ratio: f64) -> f64 {
    let s = small.as_secs_f64().max(1e-6);
    (big.as_secs_f64() / s) / size_ratio
}

#[test]
fn core_pipeline_is_linear_at_scale() {
    let sizes = [1000usize, 2000, 4000, 8000];
    let rows: Vec<Row> = sizes.iter().map(|&n| measure(n)).collect();

    println!("\n  n      parse      project    serialize  apply-op");
    for r in &rows {
        println!(
            "  {:<6} {:>8.2}ms {:>8.2}ms {:>8.2}ms {:>8.3}ms",
            r.n,
            r.parse.as_secs_f64() * 1e3,
            r.project.as_secs_f64() * 1e3,
            r.serialize.as_secs_f64() * 1e3,
            r.apply_op.as_secs_f64() * 1e3,
        );
    }

    // Compare the two largest sizes (times big enough to be signal, not noise). A linear stage's
    // per-node growth is ~1.0; assert < 1.8 so real quadratic behaviour (~2.0+) fails while leaving
    // slack for allocator/cache noise. apply-op is a single-path edit → should be ~flat (sublinear),
    // so it's reported but not ratio-gated (its absolute time is asserted tiny below).
    let (small, big) = (&rows[rows.len() - 2], &rows[rows.len() - 1]);
    let ratio = big.n as f64 / small.n as f64;
    for (name, s, b) in [
        ("parse", small.parse, big.parse),
        ("project", small.project, big.project),
        ("serialize", small.serialize, big.serialize),
    ] {
        let growth = per_node_growth(s, b, ratio);
        assert!(
            growth < 1.8,
            "{name} scaled super-linearly: {growth:.2}× per-node from n={} to n={} \
             ({:.2}ms → {:.2}ms)",
            small.n,
            big.n,
            s.as_secs_f64() * 1e3,
            b.as_secs_f64() * 1e3,
        );
    }

    // A single-path edit must not scale with document size — a whole-doc walk here would be the
    // classic O(n) hidden in an O(1) op. Generous ceiling so it's about shape, not machine speed.
    let biggest = rows.last().unwrap();
    assert!(
        biggest.apply_op < Duration::from_millis(50),
        "apply-op on an 8000-path doc took {:.3}ms — should be near-constant",
        biggest.apply_op.as_secs_f64() * 1e3
    );
}
