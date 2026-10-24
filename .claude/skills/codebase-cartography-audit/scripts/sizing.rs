//! Cartography-audit sizing: IFPUG function points, Capers Jones backfiring, and the
//! economic valuation (Modules 11 and 12). Prints ready-to-paste Markdown tables.
//!
//! Single-file, std-only Rust: build with `rustc -O --edition 2021 sizing.rs -o <bin>`.
//!
//! Usage: sizing <sizing-inputs.txt> <category-language.tsv>
//!   sizing-inputs.txt     analyst inputs (template: ../templates/sizing-inputs.txt)
//!   category-language.tsv written by `metrics` (LOC per category and language)
//!
//! Input format, one per line, `#` starts a comment, `|` separates value from note:
//!   level.EI = 1 | optional: 0 low, 1 average (default), 2 high
//!   count.EI = 180 | basis: engine command registry, state-changing commands
//!   gsc.1 = 2 | Data communications: local control socket + MCP only
//!   ratio.Rust = 55 | analyst-assumed, nearest SPR peers C++ 55 / Go 45
//!   ratio.default = 50 | for languages without their own ratio line
//!   phys_to_logical = 1.3 | scc physical lines per logical statement
//!   fp_basis = 1 | 1 = AFP, 0 = UFP: which System figure feeds the valuation
//!   salary.<n> = 133080 | source, figure, access date      (one or more)
//!   burden.low = 1.5 / burden.high = 1.75
//!   days_per_month = 22 / workdays_per_year = 260
//!   schedule_exponent = 0.42 | why this class

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

struct Inputs {
    vals: BTreeMap<String, (f64, String)>,
}

impl Inputs {
    fn get(&self, k: &str) -> Result<f64, String> {
        self.vals.get(k).map(|v| v.0).ok_or_else(|| format!("missing input `{k}`"))
    }
    fn note(&self, k: &str) -> &str {
        self.vals.get(k).map_or("", |v| v.1.as_str())
    }
    fn get_or(&self, k: &str, d: f64) -> f64 {
        self.vals.get(k).map_or(d, |v| v.0)
    }
    fn prefixed(&self, p: &str) -> Vec<(&String, &(f64, String))> {
        self.vals.iter().filter(|(k, _)| k.starts_with(p)).collect()
    }
}

const TYPES: [(&str, &str, f64, f64, f64); 5] = [
    ("EI", "External Input", 3.0, 4.0, 6.0),
    ("EO", "External Output", 4.0, 5.0, 7.0),
    ("EQ", "External Inquiry", 3.0, 4.0, 6.0),
    ("ILF", "Internal Logical File", 7.0, 10.0, 15.0),
    ("EIF", "External Interface File", 5.0, 7.0, 10.0),
];

const GSC: [&str; 14] = [
    "Data communications", "Distributed data processing", "Performance",
    "Heavily used configuration", "Transaction rate", "Online data entry",
    "End-user efficiency", "Online update", "Complex processing", "Reusability",
    "Installation ease", "Operational ease", "Multiple sites", "Facilitate change",
];

const BANDS: [(f64, &str); 5] = [
    (0.32, "Jones full-lifecycle baseline (headline)"),
    (0.5, "below baseline (large / high-ceremony)"),
    (1.0, "coding-centric / well-run"),
    (2.0, "high-performing small team"),
    (3.0, "elite / best-in-class"),
];

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(inp), Some(tsv)) = (a.get(1), a.get(2)) else {
        eprintln!("usage: sizing <sizing-inputs.txt> <category-language.tsv>");
        std::process::exit(2);
    };
    match run(inp, tsv) {
        Ok(s) => print!("{s}"),
        Err(e) => {
            eprintln!("sizing: {e}");
            std::process::exit(1);
        }
    }
}

fn parse_inputs(text: &str) -> Result<Inputs, String> {
    let mut vals = BTreeMap::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, rest)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = value", n + 1));
        };
        let (v, note) = rest.split_once('|').unwrap_or((rest, ""));
        let v: f64 = v.trim().replace(['_', ','], "").parse().map_err(|_| format!("line {}: `{}` is not a number", n + 1, v.trim()))?;
        if !v.is_finite() || v < 0.0 {
            return Err(format!("line {}: value must be finite and >= 0", n + 1));
        }
        vals.insert(k.trim().to_string(), (v, note.trim().to_string()));
    }
    Ok(Inputs { vals })
}

fn money(x: f64) -> String {
    let n = x.round().max(0.0) as u64;
    let s = n.to_string();
    let mut o = String::from("$");
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(ch);
    }
    o
}

fn num(x: f64) -> String {
    let n = x.round().max(0.0) as u64;
    let s = n.to_string();
    let mut o = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(ch);
    }
    o
}

fn run(inp_path: &str, tsv_path: &str) -> Result<String, String> {
    let inp = parse_inputs(&fs::read_to_string(inp_path).map_err(|e| format!("read {inp_path}: {e}"))?)?;
    let tsv = fs::read_to_string(tsv_path).map_err(|e| format!("read {tsv_path}: {e}"))?;
    let mut loc: BTreeMap<(String, String), f64> = BTreeMap::new();
    for line in tsv.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        if let (Some(c), Some(l), Some(v)) = (f.first(), f.get(1), f.get(2)) {
            let v: f64 = v.parse().map_err(|_| format!("bad LOC in {tsv_path}: {line}"))?;
            loc.insert((c.to_string(), l.to_string()), v);
        }
    }
    let mut s = String::new();

    // ---- IFPUG
    let _ = writeln!(s, "### IFPUG function-type table (System)\n");
    let _ = writeln!(s, "| Type | Name | Count | Complexity | Weight | FP | Basis |\n|---|---|--:|---|--:|--:|---|");
    let mut ufp = 0.0;
    let (mut tx, mut data) = (0.0, 0.0);
    for (code, name, lo, avg, hi) in TYPES {
        let key = format!("count.{code}");
        let count = inp.get(&key)?;
        let level = inp.vals.get(&format!("level.{code}")).map_or(1.0, |v| v.0);
        let (lvl, w) = match level as u32 { 0 => ("low", lo), 2 => ("high", hi), _ => ("average", avg) };
        let fp = count * w;
        ufp += fp;
        if matches!(code, "ILF" | "EIF") { data += fp } else { tx += fp }
        let _ = writeln!(s, "| {code} | {name} | {} | {lvl} | {w} | {} | {} |", num(count), num(fp), inp.note(&key));
    }
    let _ = writeln!(s, "| **UFP** | | | | | **{}** | |", num(ufp));
    if data > tx {
        let _ = writeln!(s, "\n> WARNING: data functions ({}) exceed transaction functions ({}): likely table-counting ILF/EIF; regroup into logical files.", num(data), num(tx));
    }

    let _ = writeln!(s, "\n### General System Characteristics\n");
    let _ = writeln!(s, "| # | Characteristic | DI (0-5) | Justification |\n|--:|---|--:|---|");
    let mut sum_gsc = 0.0;
    for (i, name) in GSC.iter().enumerate() {
        let key = format!("gsc.{}", i + 1);
        let v = inp.get(&key)?;
        if v > 5.0 {
            return Err(format!("{key} must be 0-5"));
        }
        sum_gsc += v;
        let _ = writeln!(s, "| {} | {name} | {v} | {} |", i + 1, inp.note(&key));
    }
    let vaf = 0.65 + 0.01 * sum_gsc;
    let afp = ufp * vaf;
    let _ = writeln!(s, "| | **ΣGSC** | **{sum_gsc}** | |");
    let _ = writeln!(s, "\nVAF = 0.65 + 0.01 x {sum_gsc} = **{vaf:.2}**; AFP = UFP x VAF = {} x {vaf:.2} = **{}**", num(ufp), num(afp));
    let use_ufp = inp.get_or("fp_basis", 1.0) == 0.0;
    let sys_ifpug = if use_ufp { ufp } else { afp };
    let _ = writeln!(s, "\nSystem FP carried into the valuation: **{}** ({}).", num(sys_ifpug), if use_ufp { "UFP" } else { "AFP" });

    // ---- Backfiring
    let p2l = inp.get_or("phys_to_logical", 1.3).max(1.0);
    let default_ratio = inp.get_or("ratio.default", 50.0).max(1.0);
    let mut cat_fp: BTreeMap<&str, (f64, f64, f64, f64)> = BTreeMap::new(); // loc, raw fp, adj fp, fp at 1.5
    for cat in ["System", "DevOps/Test"] {
        let _ = writeln!(s, "\n### Backfiring - {cat}\n");
        let _ = writeln!(s, "| Language | Physical LOC | LOC/FP | FP (raw physical) | FP (/ {p2l} physical->logical) | Ratio source |\n|---|--:|--:|--:|--:|---|");
        let (mut tl, mut raw, mut adj, mut hi) = (0.0, 0.0, 0.0, 0.0);
        for ((c, lang), v) in &loc {
            if c != cat {
                continue;
            }
            let key = format!("ratio.{lang}");
            let r = inp.vals.get(&key).map_or(default_ratio, |x| x.0).max(1.0);
            let src = if inp.vals.contains_key(&key) { inp.note(&key) } else { inp.note("ratio.default") };
            let f_raw = v / r;
            tl += v;
            raw += f_raw;
            adj += f_raw / p2l;
            hi += f_raw / 1.5;
            let _ = writeln!(s, "| {lang} | {} | {r} | {} | {} | {src} |", num(*v), num(f_raw), num(f_raw / p2l));
        }
        let _ = writeln!(s, "| **{cat} total** | **{}** | | **{}** | **{}** | |", num(tl), num(raw), num(adj));
        let _ = writeln!(s, "\nSensitivity band (physical->logical 1.5 .. 1.0): {} .. {} FP; point estimate {} FP.", num(hi), num(raw), num(adj));
        cat_fp.insert(cat, (tl, raw, adj, hi));
    }

    let (sys_loc, _, sys_bf, _) = cat_fp.get("System").copied().unwrap_or_default();
    let (_, _, dev_fp, _) = cat_fp.get("DevOps/Test").copied().unwrap_or_default();
    let div = if sys_ifpug > 0.0 { (sys_bf - sys_ifpug) / sys_ifpug * 100.0 } else { 0.0 };
    let _ = writeln!(s, "\n### System reconciliation (IFPUG vs backfiring)\n");
    let _ = writeln!(s, "| Method | System FP |\n|---|--:|\n| IFPUG ({}) | {} |\n| Backfiring (point) | {} |\n| Divergence | {:+.1}% |\n| Verdict (±30%) | {} |",
        if use_ufp { "UFP" } else { "AFP" }, num(sys_ifpug), num(sys_bf), div,
        if div.abs() <= 30.0 { "agree" } else { "REVIEW: re-examine IFPUG basis; do not scale to match" });
    let _ = writeln!(s, "\nSystem physical LOC per IFPUG FP: {:.0}", if sys_ifpug > 0.0 { sys_loc / sys_ifpug } else { 0.0 });

    let total_fp = sys_ifpug + dev_fp;
    let _ = writeln!(s, "\n### Function points: subtotals and total\n");
    let _ = writeln!(s, "| Category | Method | FP | Share |\n|---|---|--:|--:|");
    let share = |x: f64| if total_fp > 0.0 { 100.0 * x / total_fp } else { 0.0 };
    let _ = writeln!(s, "| System | IFPUG {} | {} | {:.1}% |", if use_ufp { "UFP" } else { "AFP" }, num(sys_ifpug), share(sys_ifpug));
    let _ = writeln!(s, "| DevOps/Test | backfired equivalent FP | {} | {:.1}% |", num(dev_fp), share(dev_fp));
    let _ = writeln!(s, "| **Total** | | **{}** | 100% |", num(total_fp));

    // ---- Valuation
    let salaries = inp.prefixed("salary.");
    if salaries.is_empty() {
        return Err("at least one `salary.<name> = <annual USD> | source` line is required".into());
    }
    let mut sv: Vec<f64> = salaries.iter().map(|(_, v)| v.0).collect();
    sv.sort_by(|a, b| a.total_cmp(b));
    let blended = if sv.len() % 2 == 1 { sv[sv.len() / 2] } else { (sv[sv.len() / 2 - 1] + sv[sv.len() / 2]) / 2.0 };
    let b_lo = inp.get_or("burden.low", 1.5);
    let b_hi = inp.get_or("burden.high", 1.75);
    let dpm = inp.get_or("days_per_month", 22.0).max(1.0);
    let wdy = inp.get_or("workdays_per_year", 260.0).max(1.0);
    let day_lo = blended * b_lo / wdy;
    let day_hi = blended * b_hi / wdy;

    let _ = writeln!(s, "\n### Salary inputs\n\n| Source | Annual base (USD) | Note |\n|---|--:|---|");
    for (k, v) in &salaries {
        let _ = writeln!(s, "| {} | {} | {} |", k.trim_start_matches("salary."), money(v.0), v.1);
    }
    let _ = writeln!(s, "\nBlended median base = median of the sources = **{}**; burden {b_lo}x-{b_hi}x; fully-loaded day rate = base x burden / {wdy} = **{} - {}**.", money(blended), money(day_lo), money(day_hi));

    let lpf = if sys_ifpug > 0.0 { sys_loc / sys_ifpug } else { 0.0 };
    let _ = writeln!(s, "\n### Effort by productivity band\n");
    let _ = writeln!(s, "| FP/man-day | ~FP/staff-month | Level | ~System LOC/man-day | System man-months | DevOps/Test man-months | Total man-months (coding-centric) |\n|--:|--:|---|--:|--:|--:|--:|");
    for (rate, level) in BANDS {
        let sys_mm = sys_ifpug / rate / dpm;
        let dev_mm = dev_fp / rate / dpm;
        let (dev, tot) = if rate < 0.4 { ("n/a (in System)".to_string(), "n/a (in System)".to_string()) } else { (num(dev_mm), num(sys_mm + dev_mm)) };
        let _ = writeln!(s, "| {rate} | {:.0} | {level} | {:.0} | {} | {dev} | {tot} |", rate * dpm, rate * lpf, num(sys_mm));
    }

    let _ = writeln!(s, "\n### Cost by productivity band (fully-loaded, burden {b_lo}x - {b_hi}x)\n");
    let _ = writeln!(s, "| FP/man-day | System | DevOps/Test | Total (coding-centric) |\n|--:|---|---|---|");
    let mut lo_all = f64::MAX;
    let mut hi_all: f64 = 0.0;
    for (rate, _) in BANDS {
        let sd = sys_ifpug / rate;
        let dd = dev_fp / rate;
        let sys_c = format!("{} - {}", money(sd * day_lo), money(sd * day_hi));
        lo_all = lo_all.min(sd * day_lo);
        hi_all = hi_all.max(sd * day_hi);
        let (dev_c, tot_c) = if rate < 0.4 {
            ("n/a (in System)".to_string(), "n/a (in System)".to_string())
        } else {
            (format!("{} - {}", money(dd * day_lo), money(dd * day_hi)), format!("{} - {}", money((sd + dd) * day_lo), money((sd + dd) * day_hi)))
        };
        let _ = writeln!(s, "| {rate} | {sys_c} | {dev_c} | {tot_c} |");
    }
    let head_days = sys_ifpug / 0.32;
    let best_days = (sys_ifpug + dev_fp) / 3.0;
    let _ = writeln!(s, "\n**Headline (System, full-lifecycle 0.32 FP/day): {} - {}.** Coding-centric best case (System + DevOps/Test at 3 FP/day): {} - {}. Range across all System bands: {} - {}.",
        money(head_days * day_lo), money(head_days * day_hi), money(best_days * day_lo), money(best_days * day_hi), money(lo_all), money(hi_all));

    let exp = inp.get_or("schedule_exponent", 0.42);
    let months = sys_ifpug.max(1.0).powf(exp);
    let staff_months = head_days / dpm;
    let _ = writeln!(s, "\n### Schedule sanity check\n");
    let _ = writeln!(s, "| Quantity | Value |\n|---|--:|\n| System FP | {} |\n| Exponent ({}) | {exp} |\n| Calendar months = FP^exp | {months:.1} |\n| Staff-months at 0.32 FP/day | {} |\n| Implied average team | {:.1} |",
        num(sys_ifpug), inp.note("schedule_exponent"), num(staff_months), staff_months / months.max(1.0));
    let _ = writeln!(s, "\nReplacement cost, not market value; order-of-magnitude only. Productivity: Capers Jones, \"Software Economics and Function Point Metrics\" (IFPUG, 2017) for ~7 FP/staff-month full lifecycle; Steve McConnell, \"Software Estimation\" (2006) for the 10x productivity spread.");
    Ok(s)
}
