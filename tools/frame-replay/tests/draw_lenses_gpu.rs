use keeperfx_frame_replay::draw::{Command, DrawRenderer, IMAGE, LENS_EFFECT};

fn word(bytes: &[u8], offset: &mut usize) -> u32 {
    let value = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    value
}

#[test]
#[ignore = "requires a GPU and the native lens fixture"]
fn native_lens_indices_match() {
    let path = std::env::var("KFX_LENS_FIXTURE").expect("KFX_LENS_FIXTURE is required");
    let bytes = std::fs::read(path).unwrap();
    let mut offset = 0;
    assert_eq!(word(&bytes, &mut offset), 0x534e454c);
    let count = word(&bytes, &mut offset);
    assert_eq!(count, 54);
    let mut drawing = DrawRenderer::headless().unwrap();
    for case in 0..count {
        let width = word(&bytes, &mut offset);
        let height = word(&bytes, &mut offset);
        let source_length = word(&bytes, &mut offset) as usize;
        let length = (width * height) as usize;
        let initial = &bytes[offset..offset + length];
        offset += length;
        let source = &bytes[offset..offset + source_length];
        offset += source_length;
        let expected = &bytes[offset..offset + length];
        offset += length;
        let target = drawing.create_target(width, height).unwrap();
        let initial = drawing
            .create_resource(initial, width, height, width)
            .unwrap();
        let source = drawing.create_resource(source, 1, 1, 1).unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: IMAGE,
                    width,
                    height,
                    source: initial,
                    source_width: width,
                    source_height: height,
                    ..Command::default()
                }],
            )
            .unwrap();
        drawing
            .submit(
                target,
                &[Command {
                    kind: LENS_EFFECT,
                    width,
                    height,
                    source,
                    clip_width: width,
                    clip_height: height,
                    ..Command::default()
                }],
            )
            .unwrap();
        assert_eq!(
            drawing.readback(target).unwrap(),
            expected,
            "native lens case {case}"
        );
        drawing.release_target(target).unwrap();
        drawing.release_resource(source).unwrap();
        drawing.release_resource(initial).unwrap();
    }
    assert_eq!(offset, bytes.len());
}
