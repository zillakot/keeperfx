use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, MINIMAP};
fn drawing() -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("minimap adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let renderer = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    DrawRenderer::new(&renderer, wgpu::TextureFormat::Rgba8Unorm).unwrap()
}
fn word(bytes: &[u8], offset: &mut usize) -> u32 {
    let n = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    n
}
#[test]
#[ignore = "requires GPU and actual native minimap fixture"]
fn actual_native_minimap_world_background_and_markers() {
    let bytes = std::fs::read(
        std::env::var("KFX_MINIMAP_FIXTURE").expect("KFX_MINIMAP_FIXTURE is required"),
    )
    .unwrap();
    let mut o = 0;
    let count = word(&bytes, &mut o);
    let width = word(&bytes, &mut o);
    let height = word(&bytes, &mut o);
    let size = (width * height) as usize;
    let mut draw = drawing();
    let target = draw.create_target(width, height).unwrap();
    let mut captures = 0;
    for case in 0..count {
        let length = word(&bytes, &mut o) as usize;
        let mut asset = bytes[o..o + length].to_vec();
        o += length;
        let initial = draw
            .create_resource(&bytes[o..o + size], width, height, width)
            .unwrap();
        o += size;
        draw.submit(
            target,
            &[Command {
                kind: IMAGE,
                source: initial,
                width,
                height,
                source_width: width,
                source_height: height,
                clip_width: width,
                clip_height: height,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.release_resource(initial).unwrap();
        let source = draw.create_resource(&asset, 1, 1, 1).unwrap();
        let c = Command {
            kind: MINIMAP,
            source,
            width,
            height,
            clip_width: width,
            clip_height: height,
            ..Default::default()
        };
        let mode = asset[0];
        if mode == 4 {
            captures += 1;
        }
        if case < 5 {
            let before = draw.readback(target).unwrap();
            let mut malformed = asset.clone();
            malformed[20..24].copy_from_slice(&9999u32.to_le_bytes());
            let bad = draw.create_resource(&malformed, 1, 1, 1).unwrap();
            assert!(
                draw.submit(target, &[Command { source: bad, ..c }])
                    .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
            draw.release_resource(bad).unwrap();
            assert!(
                draw.submit(
                    target,
                    &[
                        c,
                        Command {
                            kind: u32::MAX,
                            ..Default::default()
                        }
                    ]
                )
                .is_err()
            );
            assert_eq!(draw.readback(target).unwrap(), before);
        }
        asset.fill(71);
        let before_upload = draw.counters().replay;
        draw.frame_begin(target).unwrap();
        draw.submit(target, &[c]).unwrap();
        draw.frame_end().unwrap();
        let after_upload = draw.counters().replay;
        assert_eq!(
            after_upload.upload_ring_overflows - before_upload.upload_ring_overflows,
            0
        );
        if mode != 4 {
            assert!(after_upload.upload_queue_writes > before_upload.upload_queue_writes);
        }
        draw.release_resource(source).unwrap();
        let actual = draw.readback(target).unwrap();
        if let Some(i) = actual
            .iter()
            .zip(&bytes[o..o + size])
            .position(|(a, b)| a != b)
        {
            panic!(
                "case {case} mode {mode} pixel ({},{}) actual {} expected {}",
                i % width as usize,
                i / width as usize,
                actual[i],
                bytes[o + i]
            );
        }
        o += size;
    }
    assert_eq!(o, bytes.len());
    assert_eq!(captures, 8);
    assert_eq!(draw.target_resource_counters().snapshots, captures);
    draw.release_target(target).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn dictionary_reordering_missing_colours_and_partial_workgroups() {
    let mut draw = drawing();
    let width = 14u32;
    let diameter = 10u32;
    let target = draw.create_target(width, width).unwrap();
    let palette = [
        0u8, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 175, 207, 255,
    ];
    let initial: Vec<_> = (0..width * width)
        .map(|i| {
            if i % 3 == 0 {
                254
            } else {
                palette[i as usize % palette.len()]
            }
        })
        .collect();
    let image = draw.create_resource(&initial, width, width, width).unwrap();
    let cells: Vec<u16> = (0..(diameter + 1).pow(2))
        .map(|i| ((i * 617) % 38569) as u16)
        .collect();
    let mut header = [0u32; 24];
    header[0] = 4;
    header[1] = width;
    header[2] = width;
    header[3] = 2;
    header[4] = 2;
    header[5] = diameter;
    header[22] = 96;
    let capture: Vec<_> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
    let capture = draw.create_resource(&capture, 1, 1, 1).unwrap();
    let command = Command {
        kind: MINIMAP,
        source: capture,
        width,
        height: width,
        clip_width: width,
        clip_height: width,
        ..Default::default()
    };
    let disc: Vec<_> = (0..diameter)
        .flat_map(|y| (0..diameter).map(move |x| (y, x)))
        .filter(|&(y, x)| {
            let n = 25 - (5 - y as i32 - 1).pow(2);
            let s = (0..=5).filter(|v| v * v <= n).max().unwrap();
            (x as i32) >= 5 - s && (x as i32) < 5 + s
        })
        .collect();
    let lanes = disc.iter().fold(0u32, |lanes, &(y, x)| {
        lanes | 1 << (initial[((y + 2) * width + x + 2) as usize] >> 5)
    });
    assert_eq!(lanes, 0xff);
    for (count, reverse) in [
        (1usize, false),
        (2, false),
        (2, true),
        (4, true),
        (16, false),
        (16, true),
    ] {
        draw.submit(
            target,
            &[Command {
                kind: IMAGE,
                source: image,
                width,
                height: width,
                source_width: width,
                source_height: width,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.submit(target, &[command]).unwrap();
        header[0] = 0;
        header[7] = 65536;
        header[10] = diameter;
        header[11] = diameter;
        header[12] = 96;
        header[13] = 352;
        header[14] = header[13] + cells.len() as u32 * 2;
        header[15] = count as u32 * 38569;
        let dictionary: Vec<_> = (0..count)
            .map(|i| palette[if reverse { count - 1 - i } else { i }])
            .collect();
        let mut source: Vec<_> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
        source.extend(&dictionary);
        source.resize(352, 0);
        source.extend(cells.iter().flat_map(|v| v.to_le_bytes()));
        source.extend(
            (0..count)
                .flat_map(|colour| (0..38569).map(move |cell| (colour * 11 + cell * 7) as u8)),
        );
        let resource = draw.create_resource(&source, 1, 1, 1).unwrap();
        draw.submit(
            target,
            &[Command {
                source: resource,
                ..command
            }],
        )
        .unwrap();
        draw.release_resource(resource).unwrap();
        let mut expected = initial.clone();
        for &(y, x) in &disc {
            let dst = ((y + 2) * width + x + 2) as usize;
            let colour = dictionary
                .iter()
                .position(|&c| c == initial[dst])
                .unwrap_or(0);
            let cell = cells[(y * (diameter + 1) + x) as usize] as usize;
            expected[dst] = (colour * 11 + cell * 7) as u8;
        }
        assert_eq!(
            draw.readback(target).unwrap(),
            expected,
            "count={count}, reverse={reverse}"
        );
    }
    draw.release_resource(capture).unwrap();
    draw.release_resource(image).unwrap();
    draw.release_target(target).unwrap();
}

/// `ResourceKind::Minimap`: every minimap segment resolves through this arena family.
const MINIMAP_KIND: usize = 7;
const TABLE: usize = 38569;
const WIDTH: u32 = 14;
const DIAMETER: u32 = 10;
const PALETTE: [u8; 16] = [
    3, 17, 48, 96, 129, 200, 7, 23, 60, 108, 141, 212, 31, 79, 160, 240,
];

fn style_table(table: usize, salt: usize) -> Vec<u8> {
    (0..TABLE)
        .map(|cell| (table * 11 + cell * 7 + salt) as u8)
        .collect()
}

fn disc() -> Vec<(u32, u32)> {
    (0..DIAMETER)
        .flat_map(|y| (0..DIAMETER).map(move |x| (y, x)))
        .filter(|&(y, x)| {
            let n = 25 - (5 - y as i32 - 1).pow(2);
            let s = (0..=5).filter(|v| v * v <= n).max().unwrap();
            (x as i32) >= 5 - s && (x as i32) < 5 + s
        })
        .collect()
}

fn renderer(binding: Option<u64>) -> DrawRenderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: match binding {
            Some(binding) => wgpu::Limits {
                max_storage_buffer_binding_size: binding,
                ..Default::default()
            },
            None => Default::default(),
        },
        ..Default::default()
    }))
    .unwrap();
    let gpu = keeperfx_frame_replay::gpu::Renderer::new(device, queue).unwrap();
    DrawRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap()
}

struct World {
    draw: DrawRenderer,
    target: u64,
    image: u64,
    initial: Vec<u8>,
    dictionary: Vec<u8>,
    map: u32,
    header: [u32; 24],
    command: Command,
}

fn world() -> World {
    World::new(4, DIAMETER, None)
}

impl World {
    /// A minimap square whose background covers every dictionary colour, with the
    /// mode-4 snapshot captured and the header switched to the world mode. `map` is the
    /// subtile extent both axes carry, so the cell grid is `(map + 1)` squared.
    fn new(tables: usize, map: u32, binding: Option<u64>) -> Self {
        let mut draw = renderer(binding);
        let target = draw.create_target(WIDTH, WIDTH).unwrap();
        let dictionary: Vec<u8> = PALETTE[..tables].to_vec();
        let initial: Vec<u8> = (0..WIDTH * WIDTH)
            .map(|i| dictionary[i as usize % tables])
            .collect();
        let image = draw.create_resource(&initial, WIDTH, WIDTH, WIDTH).unwrap();
        let mut header = [0u32; 24];
        header[0] = 4;
        header[1] = WIDTH;
        header[2] = WIDTH;
        header[3] = 2;
        header[4] = 2;
        header[5] = DIAMETER;
        header[22] = 96;
        let capture: Vec<u8> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
        let capture = draw.create_resource(&capture, 1, 1, 1).unwrap();
        let command = Command {
            kind: MINIMAP,
            source: capture,
            width: WIDTH,
            height: WIDTH,
            clip_width: WIDTH,
            clip_height: WIDTH,
            ..Default::default()
        };
        draw.submit(
            target,
            &[Command {
                kind: IMAGE,
                source: image,
                width: WIDTH,
                height: WIDTH,
                source_width: WIDTH,
                source_height: WIDTH,
                ..Default::default()
            }],
        )
        .unwrap();
        draw.submit(target, &[command]).unwrap();
        draw.release_resource(capture).unwrap();
        header[0] = 0;
        header[7] = 65536;
        header[10] = map;
        header[11] = map;
        header[12] = 96;
        header[13] = 352;
        header[14] = header[13] + (map + 1) * (map + 1) * 2;
        header[15] = tables as u32 * TABLE as u32;
        Self {
            draw,
            target,
            image,
            initial,
            dictionary,
            map,
            header,
            command,
        }
    }

    fn cells(&self) -> Vec<u16> {
        (0..(self.map + 1) * (self.map + 1))
            .map(|i| ((i * 617) % TABLE as u32) as u16)
            .collect()
    }

    fn styles(&self, salt: usize) -> Vec<Vec<u8>> {
        (0..self.dictionary.len())
            .map(|table| style_table(table, salt))
            .collect()
    }

    fn source(&self, cells: &[u16], styles: &[Vec<u8>]) -> Vec<u8> {
        let mut source: Vec<u8> = self.header.iter().flat_map(|v| v.to_le_bytes()).collect();
        source.extend(&self.dictionary);
        source.resize(352, 0);
        source.extend(cells.iter().flat_map(|v| v.to_le_bytes()));
        for table in styles {
            source.extend(table);
        }
        source
    }

    fn expected(&self, cells: &[u16], styles: &[Vec<u8>]) -> Vec<u8> {
        let mut expected = self.initial.clone();
        for (y, x) in disc() {
            let dst = ((y + 2) * WIDTH + x + 2) as usize;
            let colour = self
                .dictionary
                .iter()
                .position(|&c| c == self.initial[dst])
                .unwrap_or(0);
            expected[dst] = styles[colour][cells[(y * (self.map + 1) + x) as usize] as usize];
        }
        expected
    }

    /// The whole validated payload, which a cold split covers segment by segment and
    /// the contiguous path covers in one resolution.
    fn payload_bytes(&self, cells: &[u16]) -> u64 {
        (96 + 256 + cells.len() * 2 + self.dictionary.len() * TABLE) as u64
    }

    /// Restores the captured background, draws one world command and returns the
    /// minimap arena deltas that command produced.
    fn round(&mut self, cells: &[u16], styles: &[Vec<u8>]) -> (u64, u64, u64) {
        self.draw
            .submit(
                self.target,
                &[Command {
                    kind: IMAGE,
                    source: self.image,
                    width: WIDTH,
                    height: WIDTH,
                    source_width: WIDTH,
                    source_height: WIDTH,
                    ..Default::default()
                }],
            )
            .unwrap();
        let before = self.draw.counters().arena_by_kind[MINIMAP_KIND];
        let source = self.source(cells, styles);
        let resource = self.draw.create_resource(&source, 1, 1, 1).unwrap();
        self.draw
            .submit(
                self.target,
                &[Command {
                    source: resource,
                    ..self.command
                }],
            )
            .unwrap();
        self.draw.release_resource(resource).unwrap();
        let after = self.draw.counters().arena_by_kind[MINIMAP_KIND];
        assert_eq!(
            self.draw.readback(self.target).unwrap(),
            self.expected(cells, styles)
        );
        (
            after.misses - before.misses,
            after.hits - before.hits,
            after.source_bytes - before.source_bytes,
        )
    }

    fn finish(mut self) {
        self.draw.release_resource(self.image).unwrap();
        self.draw.release_target(self.target).unwrap();
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn only_the_style_tables_that_changed_upload_again() {
    let mut world = world();
    let cells = world.cells();
    let tables = world.dictionary.len();
    let segments = (3 + tables) as u64;
    let mut styles = world.styles(0);

    let cold = world.round(&cells, &styles);
    assert_eq!(
        cold.0, segments,
        "every segment is cold on the first command"
    );
    assert_eq!(cold.1, 0);
    assert_eq!(
        cold.2,
        world.payload_bytes(&cells),
        "the split covers the whole validated payload once"
    );

    assert_eq!(
        world.round(&cells, &styles),
        (0, segments, 0),
        "an unchanged payload uploads nothing"
    );

    styles[2] = style_table(2, 1);
    assert_eq!(
        world.round(&cells, &styles),
        (1, segments - 1, TABLE as u64),
        "one changed table uploads one table, not the whole style set"
    );

    styles[2] = style_table(2, 0);
    assert_eq!(
        world.round(&cells, &styles),
        (0, segments, 0),
        "the retained earlier version of the table recurs"
    );

    let mut moved = cells.clone();
    moved[7] = 4321;
    assert_eq!(
        world.round(&moved, &styles),
        (1, segments - 1, (cells.len() * 2) as u64),
        "changed cells upload the cells alone"
    );

    // `h[20]` is the overlay colour the world mode never samples, so this changes
    // the prefix bytes without moving a pixel.
    world.header[20] = 5;
    assert_eq!(
        world.round(&moved, &styles),
        (1, segments - 1, 96),
        "a changed header uploads the prefix alone"
    );

    // Three prefix versions (the mode-4 capture and the two world headers), one
    // dictionary, one cell version after the replaced one retired, and five style
    // versions across the four tables.
    let counters = world.draw.counters();
    assert_eq!(
        counters.minimap_cache_cpu_bytes,
        (3 * 96 + 256 + cells.len() * 2 + 5 * TABLE) as u64
    );
    assert_eq!(
        counters.minimap_cache_class_bytes,
        (3 * 256 + 256 + 256 + 5 * 65536) * stride()
    );
    assert_eq!(counters.minimap_cache_evictions, 1, "the replaced cells");
    world.finish();
}

/// GPU bytes per arena source byte in this build.
fn stride() -> u64 {
    if cfg!(feature = "packed-arena") { 1 } else { 4 }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_aborted_frame_reuploads_every_resident_minimap_segment() {
    let mut world = world();
    let cells = world.cells();
    let segments = (3 + world.dictionary.len()) as u64;
    let styles = world.styles(0);
    world.round(&cells, &styles);
    assert_eq!(world.round(&cells, &styles), (0, segments, 0));

    let source = world.source(&cells, &styles);
    let resource = world.draw.create_resource(&source, 1, 1, 1).unwrap();
    world.draw.frame_begin(world.target).unwrap();
    world
        .draw
        .submit(
            world.target,
            &[Command {
                source: resource,
                ..world.command
            }],
        )
        .unwrap();
    world.draw.frame_abort().unwrap();
    world.draw.release_resource(resource).unwrap();

    // The content cache still holds every identity, but residency is unproven, so
    // each segment resolves to a fresh upload and the pixels stay exact.
    assert_eq!(
        world.round(&cells, &styles),
        (segments, 0, world.payload_bytes(&cells))
    );
    assert_eq!(world.round(&cells, &styles), (0, segments, 0));
    world.finish();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn every_validated_background_count_resolves_its_own_segments() {
    for tables in [1usize, 2, 11, 16] {
        let mut world = World::new(tables, DIAMETER, None);
        let cells = world.cells();
        let styles = world.styles(0);
        let segments = (3 + tables) as u64;
        assert_eq!(
            world.round(&cells, &styles),
            (segments, 0, world.payload_bytes(&cells)),
            "{tables} backgrounds cold"
        );
        assert_eq!(
            world.round(&cells, &styles),
            (0, segments, 0),
            "{tables} backgrounds warm"
        );
        world.finish();
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_world_too_large_for_the_cache_budget_stays_on_the_contiguous_payload() {
    // A 2048-subtile map carries an 8 MiB cell class on its own, past the cache
    // budget, so the command never splits and resolves as one whole payload.
    let mut world = World::new(1, 2048, None);
    let cells = world.cells();
    let styles = world.styles(0);
    let payload = world.payload_bytes(&cells);
    assert_eq!(world.round(&cells, &styles), (1, 0, payload));
    assert_eq!(world.round(&cells, &styles), (1, 0, payload));
    // Only the 96-byte mode-4 capture prefix, whose own plan fits, is resident.
    let counters = world.draw.counters();
    assert_eq!(counters.minimap_cache_cpu_bytes, 96);
    assert_eq!(counters.minimap_cache_class_bytes, 256 * stride());
    world.finish();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_disabled_arena_draws_the_minimap_through_the_batch_path() {
    // Below the arena's minimum binding the packer batches per command instead, so no
    // identity is resident and every command carries its own bytes.
    let mut world = World::new(4, DIAMETER, Some(16 << 20));
    let cells = world.cells();
    let styles = world.styles(0);
    assert_eq!(world.round(&cells, &styles), (0, 0, 0));
    assert_eq!(world.round(&cells, &styles), (0, 0, 0));
    assert_eq!(world.draw.arena_counters().bytes_uploaded, 0);
    assert_eq!(world.draw.counters().minimap_cache_cpu_bytes, 0);
    world.finish();
}
