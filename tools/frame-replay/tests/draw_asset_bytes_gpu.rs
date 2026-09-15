use keeperfx_frame_replay::draw::assets;
use wgpu::util::DeviceExt;

const READS: &str = r#"
@group(0) @binding(0) var<storage, read> assets: array<u32>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;
@compute @workgroup_size(1)
fn sample_bytes(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    output[i * 3u] = byte(i);
    output[i * 3u + 1u] = le16(i);
    output[i * 3u + 2u] = bitcast<u32>(bitcast<i32>(le32(i)));
}
"#;

#[test]
fn all_asset_modules_validate_in_both_formats() {
    for source in [
        include_str!("../src/draw_minimap.wgsl"),
        include_str!("../src/draw_shadow.wgsl"),
        include_str!("../src/draw_effects.wgsl"),
        include_str!("gpoly_pixels.wgsl"),
        READS,
    ] {
        for packed in [false, true] {
            validate(&assets::shader(source).replacen(
                &format!("= {};", assets::PACKED),
                &format!("= {packed};"),
                1,
            ));
        }
    }
    validate(include_str!("../src/draw_snapshot_pack.wgsl"));
}

fn validate(source: &str) {
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn unaligned_reads_match_expanded_and_native_bytes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut bytes = vec![0, 1, 0, 0, 1, 1, 0, 0, 0xfe, 0xff, 0xff, 0xff];
    bytes.extend((0..29).map(|i| (i * 47) as u8));
    for tail in 1..=3 {
        let data = &bytes[..32 + tail];
        let reads = data.len() - 3;
        let expected: Vec<u32> = (0..reads)
            .flat_map(|i| {
                [
                    u32::from(data[i]),
                    u32::from(u16::from_le_bytes(data[i..i + 2].try_into().unwrap())),
                    u32::from_le_bytes(data[i..i + 4].try_into().unwrap()),
                ]
            })
            .collect();
        for packed in [false, true] {
            let mut encoded: Vec<u8> = if packed {
                data.to_vec()
            } else {
                data.iter()
                    .flat_map(|&b| u32::from(b).to_le_bytes())
                    .collect()
            };
            encoded.resize(encoded.len().div_ceil(4) * 4, 0);
            let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &encoded,
                usage: wgpu::BufferUsages::STORAGE,
            });
            let size = (reads * 3 * 4) as u64;
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let source = assets::shader(READS).replacen(
                &format!("= {};", assets::PACKED),
                &format!("= {packed};"),
                1,
            );
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some("sample_bytes"),
                compilation_options: Default::default(),
                cache: None,
            });
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(reads as u32, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let actual: Vec<_> = readback
                .slice(..)
                .get_mapped_range()
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect();
            assert_eq!(actual, expected, "packed={packed}, tail={tail}");
            readback.unmap();
        }
    }
}
