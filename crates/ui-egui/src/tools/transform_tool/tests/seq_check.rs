//! Sequence checker: random walks through the open transform session (box edits in every mode, Warp in and out, warp
//! pulls, Undo and Redo) on a scaled smart object, checking invariants that need no reference:
//! - Undo then Redo (and Redo then Undo) give back exactly the state before.
//! - Box mode shows the box's own texels.
//! - Undoing every step gives back the session as it opened, and Warp after that applies what a fresh Warp applies.
//! - Cancel gives back the document as it was.
//!
//! `SEQ_SEEDS` (default 300) walks of `SEQ_LEN` (default 14) steps from `SEQ_SEED0` (default 1). A failing walk is
//! shrunk to the fewest steps that still fail and printed. Run:
//! `cargo test -p photocraft-ui-egui --lib seq_check -- --ignored --nocapture`.

use super::*;

#[derive(Clone, Debug)]
enum Step {
    Move(f64, f64),
    Scale(f64, f64),
    Turn(f64),
    Corner(usize, f64, f64),
    Perspective(f64),
    EnterWarp,
    Pull(usize, f64, f64),
    /// A drag of the surface at `(s, t)` through the real pull rule, `len` px at `angle`, frame by frame.
    LongPull(f64, f64, f64, f64),
    HoldWarp,
    LeaveWarp,
    Undo,
    Redo,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn random_step(r: &mut Rng) -> Step {
    match r.pick(11) {
        0 => Step::Move(r.range(-6.0, 6.0), r.range(-6.0, 6.0)),
        1 => Step::Scale(r.range(0.8, 1.25), r.range(0.8, 1.25)),
        2 => Step::Turn(r.range(-0.4, 0.4)),
        3 => Step::Corner(r.pick(4), r.range(-5.0, 5.0), r.range(-5.0, 5.0)),
        4 => Step::Perspective(r.range(-5.0, 5.0)),
        5 | 6 => Step::EnterWarp,
        7 => if r.pick(3) == 0 {
            Step::LongPull(r.range(0.05, 0.95), r.range(0.05, 0.95), r.range(0.0, std::f64::consts::TAU), r.range(40.0, 400.0))
        } else {
            Step::Pull(r.pick(16), r.range(-4.0, 4.0), r.range(-4.0, 4.0))
        },
        8 => if r.pick(2) == 0 { Step::HoldWarp } else { Step::LeaveWarp },
        9 => Step::Undo,
        _ => Step::Redo,
    }
}

/// What the session and its preview hold, compared exactly.
#[derive(Clone, Debug, PartialEq)]
struct Fp {
    quad: [[f64; 2]; 4],
    warp: Option<Vec<[f64; 2]>>,
    placed: Option<([f64; 9], bool)>,
    held: bool,
    other: bool,
    texels: [usize; 2],
}

fn fp(app: &PhotocraftApp) -> Fp {
    let t = open(app);
    let pv = app.transform_preview.as_ref().expect("a preview");
    Fp {
        quad: quad_of(t),
        warp: t.session.warp().map(|w| w.warp.to_mesh(1, 1).points.clone()),
        placed: pv.placed_warp,
        held: pv.held.is_some(),
        other: pv.other_texels.is_some(),
        texels: pv.textures.size(),
    }
}

/// The walk's subject, by `kind`: 0 a smart object placed at 1.5 × 1.25 of its contents (its contents' texels and its
/// box's differ in size), 1 one turned by 0.5 rad, 2 one with a stored warp (a pull applied first), 3 one placed in a
/// perspective (its top 18 px narrower).
fn subject(kind: u8) -> (Box<PhotocraftApp>, egui::Context, LayerId) {
    let mut app = app_with_square(96, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    begin(&mut app, &ctx).unwrap();
    match kind {
        0 => scale_box(&mut app, 1.5, 1.25),
        1 => turn_box(&mut app, 0.5),
        3 => {
            let q = quad_of(open(&app));
            let pivot = open(&app).session.state().pivot;
            let persp = [[q[0][0] + 9.0, q[0][1]], [q[1][0] - 9.0, q[1][1]], q[2], q[3]];
            box_events(&mut app, 1, &[BoxState::new(persp.map(|[x, y]| Point::new(x, y)), pivot)]);
        }
        _ => {
            begin_warp(&mut app, &ctx).unwrap();
            settle(&mut app);
            let mut to = warp_mesh(&app);
            to.points[6] += Vec2::new(4.0, -3.0);
            pull_mesh(&mut app, 1, to);
            settle(&mut app);
        }
    }
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    (app, ctx, id)
}

fn frame(app: &mut PhotocraftApp) {
    if app.ui.transform.is_some() {
        settle(app);
        super::super::begin::reconcile_held(app);
    }
}

fn warping(app: &PhotocraftApp) -> bool {
    open(app).session.warp().is_some()
}

fn new_quad(app: &PhotocraftApp, step: &Step) -> Option<[[f64; 2]; 4]> {
    let q = quad_of(open(app));
    let c = [(q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0, (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0];
    let map = |f: &dyn Fn([f64; 2]) -> [f64; 2]| Some(q.map(f));
    match *step {
        Step::Move(dx, dy) => map(&|p| [p[0] + dx, p[1] + dy]),
        Step::Scale(kx, ky) => map(&|p| [c[0] + (p[0] - c[0]) * kx, c[1] + (p[1] - c[1]) * ky]),
        Step::Turn(a) => map(&|p| {
            let (s, co) = a.sin_cos();
            let (x, y) = (p[0] - c[0], p[1] - c[1]);
            [c[0] + x * co - y * s, c[1] + x * s + y * co]
        }),
        Step::Corner(i, dx, dy) => {
            let mut n = q;
            n[i] = [q[i][0] + dx, q[i][1] + dy];
            Some(n)
        }
        Step::Perspective(d) => {
            let mut n = q;
            n[0][0] += d;
            n[1][0] -= d;
            Some(n)
        }
        _ => None,
    }
}

/// Applies `step` where it can apply (a box edit outside Warp, a pull inside it, …); `Ok(false)` when it didn't.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, step: &Step, gesture: &mut u64) -> Result<bool, String> {
    *gesture += 1;
    let done = match step {
        Step::Move(..) | Step::Scale(..) | Step::Turn(..) | Step::Corner(..) | Step::Perspective(..) => {
            if warping(app) {
                return Ok(false);
            }
            let Some(q) = new_quad(app, step) else { return Ok(false) };
            let pivot = open(app).session.state().pivot;
            box_events(app, *gesture, &[BoxState::new(q.map(|[x, y]| Point::new(x, y)), pivot)]);
            true
        }
        Step::EnterWarp => {
            if warping(app) {
                return Ok(false);
            }
            begin_warp(app, ctx).map_err(|e| format!("begin_warp: {e}"))?;
            true
        }
        Step::Pull(i, dx, dy) => {
            if !warping(app) {
                return Ok(false);
            }
            let mut to = warp_mesh(app);
            let k = *i % to.points.len();
            to.points[k] += Vec2::new(*dx, *dy);
            pull_mesh(app, *gesture, to);
            true
        }
        Step::LongPull(s, t, angle, len) => {
            if !warping(app) {
                return Ok(false);
            }
            let start = warp_mesh(app);
            let placed = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp);
            let maps = placed.and_then(|(m, _)| {
                let h = photocraft_algo::transform::Homography(m);
                Some((h, h.inverse()?))
            });
            let gesture = GestureId::new(*gesture);
            let mut shown = start.points.clone();
            // How often each point has passed through infinity in this drag: the reference's picture passes the
            // horizon once and settles; back and forth is the flipping that was the bug.
            let mut passes = vec![0usize; start.points.len()];
            let frames = (*len / 10.0).ceil().max(1.0) as usize;
            for f in 1..=frames {
                let d = len * f as f64 / frames as f64;
                let by = [d * angle.cos(), d * angle.sin()];
                let pulled = match &maps {
                    Some((to_doc, back)) => super::super::warp::in_contents(&start, super::super::warp::RuleEdit::Surface { s: *s, t: *t, by }, 1.0, to_doc, back),
                    None => super::super::warp::rule_mesh(&start, super::super::warp::RuleEdit::Surface { s: *s, t: *t, by }, 1.0),
                };
                let Some(pulled) = pulled else { return Ok(f > 1) };
                let mut events = EditEvents::new();
                if f == 1 {
                    events.push(EditEvent::begin(gesture, Mixed::Same(start.clone()), "Warp".into(), None));
                }
                events.push(if f == frames { EditEvent::commit(gesture, Mixed::Same(start.clone()), pulled) } else { EditEvent::update(gesture, pulled) });
                grid_edit(app, &events, false);
                // As the canvas does: busy while the pointer is down on the grid, so the drag is one undo step.
                let now_s = app.nudge_bundle.now;
                app.ui.transform.as_mut().expect("open").session.settle(now_s, f < frames);
                super::super::begin::reconcile_held(app);
                if std::env::var("SEQ_TRACE").is_ok() {
                    let pv = app.transform_preview.as_ref().expect("a preview");
                    eprintln!("    frame {f}: base {:?} placed {:?}", pv.drag_placed.map(|(m, _)| [m[6], m[7], m[8]]), pv.placed_warp.map(|(m, _)| [m[6], m[7], m[8]]));
                }
                let now = warp_mesh(app).points;
                let c = now.iter().fold(Point::ZERO, |a, p| Point::new(a.x + p.x / now.len() as f64, a.y + p.y / now.len() as f64));
                let crossed = |a: f64, b: f64| a * b < 0.0 && a.abs() > 2000.0 && b.abs() > 2000.0;
                for (i, (p, q)) in now.iter().zip(&shown).enumerate() {
                    if !(p.x.is_finite() && p.y.is_finite()) {
                        return Err(format!("frame {f} of the drag: point {i} is not finite"));
                    }
                    if crossed(p.x - c.x, q.x - c.x) || crossed(p.y - c.y, q.y - c.y) {
                        passes[i] += 1;
                        if passes[i] > 1 {
                            return Err(format!("frame {f} of the drag: point {i} flipped back through infinity: {q:?} -> {p:?}"));
                        }
                    }
                }
                shown = now;
            }
            true
        }
        Step::HoldWarp => {
            if !warping(app) {
                return Ok(false);
            }
            hold_warp(app);
            true
        }
        Step::LeaveWarp => {
            if !warping(app) {
                return Ok(false);
            }
            leave_warp(app);
            true
        }
        Step::Undo => {
            let _ = intercept(app, "edit.undo");
            true
        }
        Step::Redo => {
            let _ = intercept(app, "edit.redo");
            true
        }
    };
    frame(app);
    Ok(done)
}

fn check_now(app: &PhotocraftApp, open_texels: [usize; 2]) -> Result<(), String> {
    let f = fp(app);
    if f.quad.iter().flatten().any(|v| !v.is_finite()) {
        return Err(format!("a corner is not finite: {:?}", f.quad));
    }
    if f.warp.is_none() && !f.held && f.texels != open_texels {
        return Err(format!("box mode shows texels of size {:?}, the box's are {:?}", f.texels, open_texels));
    }
    if f.warp.is_none() && f.placed.is_some() && !f.held {
        return Err("box mode keeps a map from the contents".into());
    }
    Ok(())
}

/// The fixed ending every walk is compared on: Warp, one pull, apply. The stored warp and placement.
fn ending(app: &mut PhotocraftApp, ctx: &egui::Context, id: LayerId) -> Result<(Option<Vec<[f64; 2]>>, Option<[f64; 4]>, [f64; 6]), String> {
    begin_warp(app, ctx).map_err(|e| format!("begin_warp: {e}"))?;
    frame(app);
    let mut to = warp_mesh(app);
    to.points[5] += Vec2::new(-3.0, 2.0);
    pull_mesh(app, 9_999, to);
    frame(app);
    commit(app);
    land_jobs(app);
    if app.ui.transform.is_some() {
        return Err(format!("the ending's commit left the box open: {}", app.ui.status));
    }
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { return Err("no smart object".into()) };
    let warp = sm.warp.as_ref().and_then(|w| w.mesh.as_ref()).map(|m| m.points.clone());
    Ok((warp, sm.warp.as_ref().map(|w| w.bounds), placement_of(app, id)))
}

fn close(a: &Option<Vec<[f64; 2]>>, b: &Option<Vec<[f64; 2]>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6),
        (None, None) => true,
        _ => false,
    }
}

/// One walk; `Err` names the first invariant broken.
fn run(steps: &[Step], end: u8) -> Result<(), String> {
    let (mut app, ctx, id) = subject(end / 2);
    let before = app.session.active().unwrap().doc.layer(id).unwrap().content.clone();
    begin(&mut app, &ctx).map_err(|e| format!("begin: {e}"))?;
    frame(&mut app);
    let start = fp(&app);
    let mut gesture = 0;
    for (n, step) in steps.iter().enumerate() {
        let at = |e: String| format!("step {n} {step:?}: {e}");
        let was = fp(&app);
        let applied = apply(&mut app, &ctx, step, &mut gesture).map_err(at)?;
        if !applied {
            continue;
        }
        if std::env::var("SEQ_TRACE").is_ok() {
            eprintln!("  after {step:?}: {:?} untouched {} can_undo {}", fp(&app), open(&app).session.is_untouched(), open(&app).session.can_undo());
        }
        check_now(&app, start.texels).map_err(at)?;
        if matches!(step, Step::Undo) && fp(&app) != was {
            let undone = fp(&app);
            let _ = intercept(&mut app, "edit.redo");
            frame(&mut app);
            if fp(&app) != was {
                return Err(at(format!("Redo after it does not give back the state before:\n  before {was:?}\n  redone {:?}", fp(&app))));
            }
            let _ = intercept(&mut app, "edit.undo");
            frame(&mut app);
            if fp(&app) != undone {
                return Err(at(format!("Undo again does not give back the undone state:\n  undone {undone:?}\n  again  {:?}", fp(&app))));
            }
        }
    }
    match end % 2 {
        0 => {
            // Every step the session can take back (a held warp leaves the session "untouched" with steps to undo).
            for _ in 0..200 {
                if !open(&app).session.can_undo() {
                    break;
                }
                let _ = intercept(&mut app, "edit.undo");
                frame(&mut app);
            }
            // Warp's texels may stay stashed for a Redo, and an unchanged stored warp held under a box mode is no step
            // to undo (the ending below shows whether it applies anything); everything else is as the session opened.
            if std::env::var("SEQ_TRACE").is_ok() {
                eprintln!("  undone: {:?} untouched {} can_undo {}", fp(&app), open(&app).session.is_untouched(), open(&app).session.can_undo());
            }
            let back = Fp { other: start.other, held: start.held, ..fp(&app) };
            if back != start {
                return Err(format!("undoing every step does not give back the opened session:\n  opened {start:?}\n  now    {back:?}"));
            }
            let got = ending(&mut app, &ctx, id)?;
            let (mut fresh, fctx, fid) = subject(end / 2);
            begin(&mut fresh, &fctx).map_err(|e| format!("begin: {e}"))?;
            frame(&mut fresh);
            let want = ending(&mut fresh, &fctx, fid)?;
            if !close(&got.0, &want.0) || got.1 != want.1 || got.2.iter().zip(want.2).any(|(a, b)| (a - b).abs() > 1e-9) {
                return Err(format!("Warp after undoing everything differs from a fresh Warp:\n  got  {got:?}\n  want {want:?}"));
            }
        }
        _ => {
            cancel(&mut app);
            land_jobs(&mut app);
            let after = app.session.active().unwrap().doc.layer(id).unwrap().content.clone();
            if format!("{after:?}") != format!("{before:?}") {
                return Err("Cancel does not give back the layer as it was".into());
            }
        }
    }
    Ok(())
}

fn run_caught(steps: &[Step], end: u8) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(steps, end))) {
        Ok(r) => r,
        Err(p) => Err(format!("panic: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())),
    }
}

/// The fewest steps (removing one at a time while it still fails the same way's first line).
fn shrink(mut steps: Vec<Step>, end: u8, err: &str) -> (Vec<Step>, String) {
    // The kind of failure: its first line after the step, without numbers (they change as steps go).
    let key = |e: &str| {
        let line = e.split(':').skip(1).collect::<Vec<_>>().join(":").lines().next().unwrap_or("").to_string();
        line.chars().filter(|c| !c.is_ascii_digit() && *c != '.' && *c != '-').collect::<String>()
    };
    let want = key(err);
    let mut last = err.to_string();
    loop {
        let mut shrunk = false;
        for i in 0..steps.len() {
            let mut fewer = steps.clone();
            fewer.remove(i);
            if let Err(e) = run_caught(&fewer, end)
                && key(&e) == want
            {
                steps = fewer;
                last = e;
                shrunk = true;
                break;
            }
        }
        if !shrunk {
            return (steps, last);
        }
    }
}

#[test]
#[ignore = "trace one walk: SEQ_TRACE=1"]
fn seq_trace() {
    let steps = vec![
        Step::EnterWarp,
        Step::LongPull(0.6616269562899482, 0.4973305551415017, 4.802292692204863, 269.5955745602801),
        Step::HoldWarp,
        Step::Corner(0, -1.182275882846704, 4.621861299596933),
        Step::EnterWarp,
        Step::LongPull(0.3442365635296523, 0.6643854913256297, 1.4322961589882712, 170.276387293623),
    ];
    eprintln!("{:?}", run_caught(&steps, 4).map_err(|e| e.lines().next().unwrap_or("").to_string()));
}

#[test]
#[ignore = "long: run on demand (see the module docs)"]
fn seq_check() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (seeds, len, seed0) = (env("SEQ_SEEDS", 300), env("SEQ_LEN", 14) as usize, env("SEQ_SEED0", 1));
    let mut failures = std::collections::BTreeMap::<String, (u64, Vec<Step>, u8, String)>::new();
    let only: Vec<u64> = std::env::var("SEQ_ONLY").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
    let all: Vec<u64> = if only.is_empty() { (seed0..seed0 + seeds).collect() } else { only };
    for seed in all {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let steps: Vec<Step> = (0..len).map(|_| random_step(&mut r)).collect();
        let end = (seed % 8) as u8;
        if let Err(e) = run_caught(&steps, end) {
            let (steps, e) = shrink(steps, end, &e);
            let kind = e.split(':').skip(1).collect::<Vec<_>>().join(":").lines().next().unwrap_or("").trim().chars().filter(|c| !c.is_ascii_digit() && *c != '.' && *c != '-').collect::<String>();
            failures.entry(kind).or_insert((seed, steps, end, e));
        }
    }
    for (kind, (seed, steps, end, e)) in &failures {
        eprintln!("\n=== {kind}\n  seed {seed}, subject {}, end {}\n  steps {steps:?}\n  {e}", end / 2, if end % 2 == 0 { "undo-all + Warp" } else { "Cancel" });
    }
    assert!(failures.is_empty(), "{} kinds of failure", failures.len());
}
