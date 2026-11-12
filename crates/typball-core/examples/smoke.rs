fn main() {
    let t = std::time::Instant::now();
    typball_core::warm_up();
    println!("warm_up: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let out = typball_core::compile(typball_core::SAMPLE_SOURCE, None, 2.0);
    println!(
        "compile: {:?} ok={} pages={} diags={:?}",
        t.elapsed(),
        out.ok,
        out.pages.len(),
        out.diagnostics
    );
    for p in &out.pages {
        println!("page {}x{}", p.width, p.height);
    }
    let out = typball_core::compile(
        "#set text(font: \"Yu Gothic UI\")\nこんにちは #unknown()",
        None,
        1.0,
    );
    println!("err test: ok={} diags={:?}", out.ok, out.diagnostics);
}
