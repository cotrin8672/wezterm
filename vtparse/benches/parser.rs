use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vtparse::{CsiParam, VTActor, VTParser};

#[derive(Default)]
struct CountingActor {
    events: usize,
}

impl CountingActor {
    #[inline]
    fn record(&mut self) {
        self.events = self.events.wrapping_add(1);
    }
}

impl VTActor for CountingActor {
    fn print(&mut self, _b: char) {
        self.record();
    }

    fn execute_c0_or_c1(&mut self, _control: u8) {
        self.record();
    }

    fn dcs_hook(
        &mut self,
        _mode: u8,
        _params: &[i64],
        _intermediates: &[u8],
        _ignored_excess_intermediates: bool,
    ) {
        self.record();
    }

    fn dcs_put(&mut self, _byte: u8) {
        self.record();
    }

    fn dcs_unhook(&mut self) {
        self.record();
    }

    fn esc_dispatch(
        &mut self,
        _params: &[i64],
        _intermediates: &[u8],
        _ignored_excess_intermediates: bool,
        _byte: u8,
    ) {
        self.record();
    }

    fn csi_dispatch(&mut self, _params: &[CsiParam], _parameters_truncated: bool, _byte: u8) {
        self.record();
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]]) {
        self.record();
    }

    fn apc_dispatch(&mut self, _data: Vec<u8>) {
        self.record();
    }
}

fn repeated_sequence(sequence: &[u8], count: usize) -> Vec<u8> {
    let mut input = Vec::with_capacity(sequence.len() * count);
    for _ in 0..count {
        input.extend_from_slice(sequence);
    }
    input
}

fn bench_parser(c: &mut Criterion) {
    let osc = repeated_sequence(b"\x1b]0;wezterm\x07", 1_000);
    let apc = repeated_sequence(b"\x1b_ wezterm\x1b\\", 1_000);

    c.bench_function("vtparse/osc_repeated", |b| {
        b.iter(|| {
            let mut parser = VTParser::new();
            let mut actor = CountingActor::default();
            parser.parse(black_box(&osc), &mut actor);
            black_box(actor.events);
        })
    });

    c.bench_function("vtparse/apc_repeated", |b| {
        b.iter(|| {
            let mut parser = VTParser::new();
            let mut actor = CountingActor::default();
            parser.parse(black_box(&apc), &mut actor);
            black_box(actor.events);
        })
    });

    // Keep one parser alive so this measures the long-lived buffer behavior that
    // matters for terminal sessions, including the giant-payload cleanup path.
    let mut large_osc = Vec::with_capacity(128 * 1024);
    large_osc.extend_from_slice(b"\x1b]0;");
    large_osc.extend(std::iter::repeat_n(b'x', 128 * 1024));
    large_osc.extend_from_slice(b"\x07");
    let small_osc = b"\x1b]0;x\x07";

    c.bench_function("vtparse/osc_large_then_small", |b| {
        let mut parser = VTParser::new();
        let mut actor = CountingActor::default();
        b.iter(|| {
            parser.parse(black_box(&large_osc), &mut actor);
            parser.parse(black_box(small_osc), &mut actor);
            debug_assert!(parser.is_ground());
            black_box(actor.events);
            actor.events = 0;
        })
    });
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
