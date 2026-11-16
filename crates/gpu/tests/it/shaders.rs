//! The compositor's shaders parse and validate (naga, as wgpu does), with no GPU: an invalid
//! operator shader would otherwise only show as operators being unavailable, and the operator
//! parity tests skip on such a device.

fn validate(name: &str, src: &str) {
    let module = match wgpu::naga::front::wgsl::parse_str(src) {
        Ok(m) => m,
        Err(e) => panic!("{name}: {}", e.emit_to_string(src)),
    };
    let mut v = wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all());
    if let Err(e) = v.validate(&module) {
        panic!("{name}: {}", e.emit_to_string(src));
    }
}

#[test]
fn the_operator_shader_validates() {
    validate("operator.wgsl", include_str!("../../src/operator.wgsl"));
}
