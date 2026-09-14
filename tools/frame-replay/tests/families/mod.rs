#![allow(dead_code)]
use keeperfx_frame_replay::draw::{
    ABI_VERSION, BITMAP, CIRCLE_FILLED, CIRCLE_OUTLINE, CLEAR, Command, DrawRenderer, GPOLY_SPAN,
    IMAGE, LENS_EFFECT, MAP_VIEW, MINIMAP, MOVIE, OPAQUE, RAW_IMAGE, RECT, SPRITE, TILED_IMAGE,
    TRIG, TriangleCommand,
};
use keeperfx_frame_replay::gpoly::Vertex;

pub struct Assets {
    image: u64,
    sprite: u64,
    texture: u64,
    shades: u64,
    tiles: u64,
    movie: u64,
    styles: u64,
    markers: u64,
    huge: u64,
    geometry: u64,
    fades: u64,
    pub ordered: u64,
    pub terrain: u64,
    pub terrain_fade: u64,
}

pub fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn sprite_bytes(w: usize, h: usize, origin: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            bytes.push((y * w + x) as u8 + 3);
            bytes.push(1);
        }
    }
    for axis in [w, h] {
        for i in 0..axis {
            bytes.extend(words(&[origin + 2 * i as u32, 2]));
        }
    }
    bytes.extend((0..256).map(|k: u32| ((k * 7 + 40) % 256) as u8));
    bytes
}

fn huge_bytes() -> Vec<u8> {
    let mut bytes = words(&[1, 1, 32, 1]);
    bytes.extend(words(&[2, 1, 44, 1]));
    bytes.extend(words(&[1, 3, 77]));
    bytes.extend(words(&[2, 3, 88]));
    bytes
}

fn blob(drawing: &mut DrawRenderer, bytes: &[u8]) -> u64 {
    let length = bytes.len() as u32;
    drawing.create_resource(bytes, length, 1, length).unwrap()
}

pub fn assets(drawing: &mut DrawRenderer) -> Assets {
    let image: Vec<u8> = (0..12).map(|i| 100 + i as u8).collect();
    let texture: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
    let shades: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let raw: Vec<u8> = (0..16).map(|i| 150 + i as u8).collect();
    let movie: Vec<u8> = (0..32).map(|i| 60 + i as u8).collect();
    let mut styles = words(&[10, 20]);
    styles.truncate(8);
    styles.extend((0..1280).map(|k: u32| (k % 199) as u8));
    let mut geometry = words(&[2, 2, 0, 0, 0]);
    geometry.extend(words(&[12, 3, 0, 0, 0]));
    geometry.extend(words(&[4, 12, 0, 0, 0]));
    let fades: Vec<u8> = (0..256 * 320).map(|i| (i % 256) as u8).collect();
    let mut ordered = vec![77, 2];
    ordered.extend(words(&[2, 2, 2, 2]));
    ordered.extend(0..=255);
    let terrain_fade: Vec<u8> = (0..16384).map(|i| i as u8).collect();
    Assets {
        image: drawing.create_resource(&image, 4, 3, 4).unwrap(),
        sprite: blob(drawing, &sprite_bytes(2, 2, 4)),
        texture: drawing.create_resource(&texture, 32, 32, 256).unwrap(),
        shades: drawing.create_resource(&shades, 256, 256, 256).unwrap(),
        tiles: drawing.create_resource(&raw, 4, 4, 4).unwrap(),
        movie: drawing.create_resource(&movie, 8, 4, 8).unwrap(),
        styles: blob(drawing, &styles),
        markers: drawing
            .create_resource(&words(&[0, 0, 1, 1]), 16, 1, 16)
            .unwrap(),
        huge: blob(drawing, &huge_bytes()),
        geometry: drawing.create_resource(&geometry, 60, 1, 60).unwrap(),
        fades: drawing.create_resource(&fades, 256, 320, 256).unwrap(),
        ordered: blob(drawing, &ordered),
        terrain: drawing
            .create_resource(&vec![37; 8192], 32, 32, 256)
            .unwrap(),
        terrain_fade: drawing
            .create_resource(&terrain_fade, 256, 64, 256)
            .unwrap(),
    }
}

/// One command of every kind the raster stream carries, sized to the view it is
/// issued against, so every sampler runs at the view's origin.
pub fn family(a: &Assets, width: u32, height: u32, tint: u32) -> Vec<Command> {
    vec![
        Command {
            kind: CLEAR,
            colour: tint,
            ..Default::default()
        },
        Command {
            kind: RECT,
            colour: tint + 1,
            x: 1,
            y: 1,
            width: 5,
            height: 4,
            ..Default::default()
        },
        Command {
            kind: IMAGE,
            source: a.image,
            x: 2,
            y: 3,
            width: 8,
            height: 6,
            source_width: 4,
            source_height: 3,
            ..Default::default()
        },
        Command {
            kind: GPOLY_SPAN,
            source: a.texture,
            table: a.shades,
            x: 3,
            y: 6,
            width: 9,
            height: 1,
            step_high: 1,
            transparent: OPAQUE,
            ..Default::default()
        },
        Command {
            kind: CIRCLE_FILLED,
            colour: tint + 2,
            x: 8,
            y: 8,
            width: 7,
            height: 7,
            source_width: 3,
            ..Default::default()
        },
        Command {
            kind: CIRCLE_OUTLINE,
            colour: tint + 3,
            x: 1,
            y: 8,
            width: 9,
            height: 9,
            source_width: 4,
            ..Default::default()
        },
        Command {
            kind: SPRITE,
            source: a.sprite,
            width,
            height,
            source_width: 2,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: TILED_IMAGE,
            source: a.tiles,
            x: 5,
            y: 10,
            width: 11,
            height: 5,
            source_width: 4,
            source_height: 4,
            ..Default::default()
        },
        Command {
            kind: RAW_IMAGE,
            source: a.tiles,
            width,
            height,
            source_width: 4,
            source_height: 4,
            step_low: width,
            step_high: height,
            ..Default::default()
        },
        Command {
            kind: MOVIE,
            source: a.movie,
            width,
            height,
            source_width: 8,
            source_height: 4,
            ..Default::default()
        },
        Command {
            kind: MAP_VIEW,
            source: a.styles,
            x: 1,
            y: 12,
            width: 8,
            height: 2,
            source_width: 4,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: MAP_VIEW,
            source: a.markers,
            colour: tint + 4,
            width,
            height,
            source_x: 3,
            source_y: 2,
            start_low: 5,
            start_high: 5,
            step_low: 1,
            step_high: 1,
            ..Default::default()
        },
        Command {
            kind: BITMAP,
            source: a.huge,
            width,
            height,
            source_height: 2,
            ..Default::default()
        },
        Command {
            kind: TRIG,
            source: a.geometry,
            table: a.fades,
            colour: tint + 5,
            width,
            height,
            source_width: 64,
            transparent: OPAQUE,
            ..Default::default()
        },
    ]
}

/// Serial row copies; the record keeps its own single-workgroup pass.
pub fn ordered_sprite(a: &Assets, width: u32, height: u32) -> Command {
    Command {
        kind: SPRITE,
        source: a.ordered,
        source_x: 9,
        source_width: 1,
        source_height: 1,
        width,
        height,
        clip_width: width,
        clip_height: height,
        ..Default::default()
    }
}

/// The alias lens reads the target it writes, so it keeps its own pass.
pub fn alias_lens(drawing: &mut DrawRenderer, width: u32, height: u32) -> u64 {
    let extent = width * height;
    let header = [
        2,
        width,
        height,
        width,
        width,
        0,
        1,
        65536 / width,
        65536 / height,
        128,
        0,
        64,
        64 + extent,
        65 + extent,
        1,
        1,
    ];
    let mut bytes = words(&header);
    bytes.extend(std::iter::repeat_n(0u8, extent as usize + 1));
    blob(drawing, &bytes)
}

pub fn lens_command(source: u64, width: u32, height: u32) -> Command {
    Command {
        kind: LENS_EFFECT,
        source,
        width,
        height,
        clip_width: width,
        clip_height: height,
        ..Default::default()
    }
}

pub fn minimap(drawing: &mut DrawRenderer, width: u32, height: u32) -> u64 {
    let mut header = [0u32; 24];
    header[0] = 2;
    header[1] = width;
    header[2] = height;
    header[3] = 1;
    header[4] = 1;
    header[5] = 8;
    header[18] = 4;
    header[20] = 200;
    header[21] = 100;
    header[22] = 96;
    header[23] = 2;
    let mut bytes = words(&header);
    bytes.extend(words(&[3, 5, 6, 2]));
    blob(drawing, &bytes)
}

pub fn minimap_command(source: u64, width: u32, height: u32) -> Command {
    Command {
        kind: MINIMAP,
        source,
        width,
        height,
        clip_width: width,
        clip_height: height,
        ..Default::default()
    }
}

pub fn terrain_triangle(a: &Assets, shade: i64) -> TriangleCommand {
    TriangleCommand {
        abi_version: ABI_VERSION,
        reserved: 0,
        source: a.terrain,
        table: a.terrain_fade,
        vertices: [(1, 1), (7, 2), (2, 7)].map(|(x, y)| Vertex {
            x,
            y,
            u: 0,
            v: 0,
            shade,
        }),
    }
}
