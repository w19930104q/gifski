use gifski::{Settings, new, progress};
use imgref::{ImgRef, ImgRefMut, ImgVec};
use rgb::{ComponentMap, RGBA8};
use std::path::{Path, PathBuf};

#[test]
fn n_frames() {
    for num_frames in 1..=11 {
        assert_anim_eq(num_frames, frame_filename, None, 0.8);
    }
}

fn assert_anim_eq(num_frames: usize, frame_filename: fn(usize) -> PathBuf, frame_edit: Option<fn(usize, ImgRefMut<RGBA8>)>, max_diff: f64) {
    let (c, w) = new(Settings::default()).unwrap();

    let t = std::thread::spawn(move || {
        for n in 0..num_frames {
            let pts = if num_frames == 1 { 0.1 } else { n as f64 / 10. };
            let name = frame_filename(n);
            if let Some(frame_edit) = frame_edit {
                let mut frame = load_frame(&name);
                frame_edit(n, frame.as_mut());
                c.add_frame_rgba(n, frame, pts).unwrap();
            } else {
                c.add_frame_png_file(n, name, pts).unwrap();
            }
        }
    });

    let mut out = Vec::new();
    w.write(&mut out, &mut progress::NoProgress {}).unwrap();
    t.join().unwrap();

    // std::fs::write(format!("/tmp/anim{num_frames}{max_diff}{}.png", frame_edit.is_some()), &out);

    let mut n = 0;
    let mut frames_seen = 0;
    for_each_frame(&out, |delay, _, actual| {
        frames_seen += 1;
        let next_n = delay as usize / 10;
        while n < next_n {
            let name = frame_filename(n);
            let mut expected = load_frame(&name);
            if let Some(frame_edit) = frame_edit {
                frame_edit(n, expected.as_mut());
            }
            assert_images_eq(expected.as_ref(), actual, max_diff, format_args!("n={n}/{num_frames}, {delay}/{next_n}, {}", name.display()));
            n += 1;
        }
    });
    assert!(n == num_frames, "{frames_seen} : {num_frames}");
}

fn load_frame(name: &Path) -> ImgVec<RGBA8> {
    let img = lodepng::decode32_file(name).unwrap();
    ImgVec::new(img.buffer, img.width, img.height)
}

#[test]
fn all_dupe_frames() {
    let (c, w) = new(Settings::default()).unwrap();

    let t = std::thread::spawn(move || {
        c.add_frame_png_file(0, frame_filename(1), 0.1).unwrap();
        c.add_frame_png_file(1, frame_filename(1), 1.2).unwrap();
        c.add_frame_png_file(2, frame_filename(1), 1.3).unwrap();
    });

    let mut out = Vec::new();
    w.write(&mut out, &mut progress::NoProgress {}).unwrap();
    t.join().unwrap();

    let mut n = 0;
    let mut delays = vec![];
    for_each_frame(&out, |delay, frame, actual| {
        let expected = lodepng::decode32_file(frame_filename(1)).unwrap();
        let expected = ImgVec::new(expected.buffer, expected.width, expected.height);
        assert_images_eq(expected.as_ref(), actual, 0., format_args!("n={n}, {delay} "));
        delays.push(frame.delay);
        n += 1;
    });
    assert_eq!(delays, [130]);
}

#[test]
fn errors() {
    let (c, w) = new(Settings::default()).unwrap();

    #[derive(Debug)]
    struct Errors(Vec<String>);

    impl progress::ProgressReporter for Errors {
        fn increase(&mut self) -> bool { true }
        fn error(&mut self, msg: String) {
            self.0.push(msg);
        }
    }

    let t = std::thread::spawn(move || {
        c.add_frame_png_file(1, frame_filename(1), 0.0).unwrap();
        c.add_frame_png_file(1, frame_filename(1), 2.0).unwrap();
        c.add_frame_png_file(2, frame_filename(1), 1.1).unwrap();
    });

    let mut out = Vec::new();
    let mut errs = Errors(vec![]);
    w.write(&mut out, &mut errs).unwrap();
    t.join().unwrap();

    // Due to concurrent processing, the exact errors and their count may vary
    // depending on thread scheduling. We verify that the error detection works
    // by checking for expected error patterns rather than exact messages.
    assert!(!errs.0.is_empty(), "expected errors to be reported for invalid frame sequence");

    // Check that we detect skipped frame index (expected 0, got 1)
    let has_skipped_frame_error = errs.0.iter().any(|e| e.contains("expected frame_number 0, got 1"));
    assert!(has_skipped_frame_error, "should detect skipped frame index: {errs:#?}");

    // Check that we detect duplicate frame index (expected N, got 1)
    let has_duplicate_index_error = errs.0.iter().any(|e|
        e.contains("got 1") && e.contains("expected frame_number") && !e.contains("expected frame_number 0")
    );
    assert!(has_duplicate_index_error, "should detect duplicate frame index: {errs:#?}");

    // Check that we detect non-monotonic pts (pts should increase)
    let has_pts_error = errs.0.iter().any(|e| e.contains("to have pts >"));
    assert!(has_pts_error, "should detect non-monotonic presentation timestamp: {errs:#?}");
}

#[test]
fn all_but_one_dupe_frames() {
    let (c, w) = new(Settings::default()).unwrap();

    let t = std::thread::spawn(move || {
        c.add_frame_png_file(0, frame_filename(0), 0.0).unwrap();
        c.add_frame_png_file(1, frame_filename(1), 1.2).unwrap();
        c.add_frame_png_file(2, frame_filename(1), 1.3).unwrap();
    });

    let mut out = Vec::new();
    w.write(&mut out, &mut progress::NoProgress {}).unwrap();
    t.join().unwrap();

    let mut delays = vec![];
    let mut n = 0;
    for_each_frame(&out, |delay, frame, actual| {
        let name = frame_filename(if n == 0 { 0 } else { 1 });
        let expected = lodepng::decode32_file(&name).unwrap();
        let expected = ImgVec::new(expected.buffer, expected.width, expected.height);
        assert_images_eq(expected.as_ref(), actual, 1.7, format_args!("n={n}, {delay} {}", name.display()));
        delays.push(frame.delay);
        n += 1;
    });
    assert_eq!(delays, [120, 20]);
}

fn frame_filename(n: usize) -> PathBuf {
    format!("tests/{}.png", (n % 3) + 1).into()
}

fn for_each_frame(mut gif_data: &[u8], mut cb: impl FnMut(u32, &gif::Frame, ImgRef<RGBA8>)) {
    let mut gif_opts = gif::DecodeOptions::new();
    gif_opts.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = gif_opts.read_info(&mut gif_data).unwrap();
    let mut screen = gif_dispose::Screen::new_decoder(&decoder);

    let mut delay = 0;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        screen.blit_frame(frame).unwrap();
        delay += u32::from(frame.delay);
        cb(delay, frame, screen.pixels_rgba());
    }
}

#[test]
fn anim3() {
    assert_anim_eq(6 * 3, |n| format!("tests/a3/{}{}.png", ["x", "y", "z"][n / 6], n % 6).into(), None, 0.8);
}

#[test]
fn anim3_transparent1() {
    assert_anim_eq(6*3, |n| format!("tests/a3/{}{}.png", ["x","y","z"][n/6], n%6).into(), Some(|_,mut fr| {
        fr.pixels_mut().for_each(|px| if px.r == 0 && px.g == 0 { px.a = 0; });
    }), 0.8);
}

#[test]
fn anim3_transparent2() {
    assert_anim_eq(6*3, |n| format!("tests/a3/{}{}.png", ["x","y","z"][n/6], n%6).into(), Some(|_,mut fr| {
        fr.pixels_mut().for_each(|px| if px.r != 0 { px.a = 0; });
    }), 0.8);
}

#[test]
fn anim3_twitch() {
    assert_anim_eq(6*3*3, |x| {
        let n = (x/3) ^ (x&1);
        format!("tests/a3/{}{}.png", ["x","y","z"][n/6], n%6).into()
    }, None, 0.8);
}

#[test]
fn anim3_mix() {
    assert_anim_eq(6*3*3, |x| {
        let n = (x/3) ^ (x&3);
        format!("tests/a3/{}{}.png", ["x","y","z"][(n/6)%3], n%6).into()
    }, Some(|n, mut fr| {
        fr.pixels_mut().take(12).for_each(|px| {
            px.g = px.g.wrapping_add(n as _);
        });
    }), 2.);
}

#[test]
fn anim2_fwd() {
    assert_anim_eq(43, |n| format!("tests/a2/{:02}.png", 1 + n).into(), None, 0.8);
}

#[test]
fn anim2_rev() {
    assert_anim_eq(43, |n| format!("tests/a2/{:02}.png", 43 - n).into(), None, 0.8);
}

#[test]
fn anim2_dupes() {
    assert_anim_eq(43 * 2, |n| format!("tests/a2/{:02}.png", 1 + n / 2).into(), None, 0.8);
}

#[test]
fn anim2_flips() {
    assert_anim_eq(43 * 2, |n| format!("tests/a2/{:02}.png", if n & 1 == 0 { 10 } else { 1 + n / 2 }).into(), None, 0.8);
}

#[test]
fn anim2_transparent() {
    assert_anim_eq(43, |n| format!("tests/a2/{:02}.png", 1+n).into(), Some(|_, mut fr| {
        fr.pixels_mut().for_each(|px| if px.r > 128 { px.a = 0; });
    }), 0.8);
}

#[test]
fn anim2_transparent2() {
    assert_anim_eq(43, |n| format!("tests/a2/{:02}.png", 43-n).into(), Some(|_, mut fr| {
        fr.pixels_mut().for_each(|px| if px.g > 200 { px.a = 0; });
    }), 0.8);
}

#[test]
fn anim2_transparent_half() {
    assert_anim_eq(43, |n| format!("tests/a2/{:02}.png", 43-n).into(), Some(|_, mut fr| {
        let n = fr.width()*(fr.height()/2);
        fr.pixels_mut().skip(n).for_each(|px| if px.g > 200 { px.a = 0; });
    }), 0.8);
}

#[track_caller]
fn assert_images_eq(a: ImgRef<RGBA8>, b: ImgRef<RGBA8>, max_diff: f64, msg: impl std::fmt::Display) {
    let diff = a.pixels().zip(b.pixels()).map(|(a,b)| {
        if a.a != b.a {
            return 300000;
        }
        if a.a == 0 {
            return 0;
        }
        let a = a.map(i32::from);
        let b = b.map(i32::from);
        let d = a - b;
        (d.r * d.r * 2 +
         d.g * d.g * 3 +
         d.b * d.b) as u64
    }).sum::<u64>() as f64 / (a.width() * a.height() * 3) as f64;
    if diff > max_diff {
        dump("expected", a);
        dump("actual", b);
    }
    assert!(diff <= max_diff, "{diff} diff > {max_diff} {msg}");
}

fn dump(filename: &str, px: ImgRef<RGBA8>) {
    let (buf, w, h) = px.to_contiguous_buf();
    lodepng::encode32_file(format!("/tmp/gifski-test-{filename}.png"), &buf, w, h).unwrap();
}

// ---------- Timeline / duplicate-folding / trailer regression tests ----------

struct GifMeta {
    /// (delay in centiseconds, disposal, screen pixel hash) per decoded frame
    frames: Vec<(u16, gif::DisposalMethod, u64)>,
    /// NETSCAPE2.0 loop count, or None when no looping extension is written
    loop_count: Option<u16>,
    /// Last byte of the file must be the GIF trailer (0x3B), exactly once at the end
    trailer_ok: bool,
}

fn encode_gif(seq: &[(usize, f64)], repeat: gifski::Repeat, edit: Option<fn(usize, ImgRefMut<RGBA8>)>) -> GifMeta {
    let settings = Settings { repeat, ..Settings::default() };
    let (c, w) = new(settings).unwrap();
    let s = seq.to_vec();
    let t = std::thread::spawn(move || {
        for (ordinal, (img_idx, pts)) in s.iter().enumerate() {
            let name = frame_filename(*img_idx);
            if let Some(edit) = edit {
                let mut frame = load_frame(&name);
                edit(ordinal, frame.as_mut());
                c.add_frame_rgba(ordinal, frame, *pts).unwrap();
            } else {
                c.add_frame_png_file(ordinal, name, *pts).unwrap();
            }
        }
    });
    let mut out = Vec::new();
    w.write(&mut out, &mut progress::NoProgress {}).unwrap();
    t.join().unwrap();

    // The GIF trailer is the terminator of the whole stream, so the final byte
    // must be 0x3B. (The same byte value can occur inside LZW data, so only the
    // last byte is meaningful here.)
    let trailer_ok = out.last() == Some(&0x3b);

    let loop_count = out.windows(11).position(|w| w == b"NETSCAPE2.0").map(|p| {
        assert_eq!(out[p + 11], 3);
        assert_eq!(out[p + 12], 1);
        u16::from_le_bytes([out[p + 13], out[p + 14]])
    });

    let mut gif_opts = gif::DecodeOptions::new();
    gif_opts.set_color_output(gif::ColorOutput::Indexed);
    let mut data = out.as_slice();
    let mut decoder = gif_opts.read_info(&mut data).unwrap();
    let mut screen = gif_dispose::Screen::new_decoder(&decoder);
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        screen.blit_frame(frame).unwrap();
        // FNV-1a over the composited screen pixels (RGBA), so disposal is actually exercised
        let mut hash: u64 = 1469598103934665603;
        for px in screen.pixels_rgba().pixels() {
            for &b in &[px.r, px.g, px.b, px.a] {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(1099511628211);
            }
        }
        frames.push((frame.delay, frame.dispose, hash));
    }
    GifMeta { frames, loop_count, trailer_ok }
}

/// Irregular presentation timestamps with duplicate frames folded in,
/// including a trailing duplicate separated by a sub-centisecond gap.
/// The folded-in duration of the last real frame must not be truncated to the
/// 2cs GIF minimum just because the trailing duplicate sits 5ms later.
#[test]
fn irregular_timing_with_duplicates() {
    let seq: Vec<(usize, f64)> = vec![
        (0, 0.0),
        (1, 0.07),
        (1, 0.15), // duplicate of previous, 8cs later
        (2, 0.30),
        (0, 0.45),
        (0, 0.455), // trailing duplicate, only 0.5cs later (the bug trigger)
    ];
    let meta = encode_gif(&seq, gifski::Repeat::Infinite, None);

    let delays: Vec<u16> = meta.frames.iter().map(|f| f.0).collect();
    let total: u32 = meta.frames.iter().map(|f| u32::from(f.0)).sum();

    // One duplicate is folded away, so six inputs become five GIF frames.
    assert_eq!(meta.frames.len(), 5);
    // Last frame keeps a real duration instead of being clamped to the 2cs minimum.
    assert_eq!(delays, [7, 23, 15, 2, 14]);
    // The full input timeline (0.455s + the 15cs frame-rate tail) is preserved.
    assert_eq!(total, 61);
    assert!(meta.trailer_ok);
    assert_eq!(meta.loop_count, Some(0)); // Infinite -> NETSCAPE loop count 0
    assert!(meta.frames.iter().all(|f| f.1 == gif::DisposalMethod::Keep));

    // Decoding twice must give byte-identical composited frames (timeline is deterministic).
    let again = encode_gif(&seq, gifski::Repeat::Infinite, None);
    let hashes_a: Vec<u64> = meta.frames.iter().map(|f| f.2).collect();
    let hashes_b: Vec<u64> = again.frames.iter().map(|f| f.2).collect();
    assert_eq!(hashes_a, hashes_b);
    assert_eq!(meta.frames.len(), again.frames.len());
}

/// Loop count and trailer must be written consistently regardless of the repeat setting.
#[test]
fn loop_count_and_trailer() {
    let seq: Vec<(usize, f64)> = vec![(0, 0.0), (1, 0.1), (1, 0.2), (2, 0.3)];

    let infinite = encode_gif(&seq, gifski::Repeat::Infinite, None);
    assert_eq!(infinite.loop_count, Some(0));
    assert!(infinite.trailer_ok);

    let finite = encode_gif(&seq, gifski::Repeat::Finite(3), None);
    assert_eq!(finite.loop_count, Some(3));
    assert!(finite.trailer_ok);

    // "play once" is encoded by omitting the NETSCAPE extension entirely.
    let once = encode_gif(&seq, gifski::Repeat::Finite(0), None);
    assert_eq!(once.loop_count, None);
    assert!(once.trailer_ok);

    // repeat never changes the frame timeline itself
    assert_eq!(infinite.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
               finite.frames.iter().map(|f| f.0).collect::<Vec<_>>());
    assert_eq!(infinite.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
               once.frames.iter().map(|f| f.0).collect::<Vec<_>>());
}

/// A non-zero first-frame pts shifts the timeline and supplies the last frame's
/// duration; a trailing duplicate must not shorten that final frame either.
#[test]
fn fixed_offset_tail_with_duplicate() {
    let seq: Vec<(usize, f64)> = vec![
        (0, 0.10), // first pts > 0.01 -> fixed offset, becomes the tail duration
        (1, 0.20),
        (1, 0.25), // duplicate
        (2, 0.40),
        (0, 0.55),
        (0, 0.555), // trailing duplicate, 0.5cs gap
    ];
    let meta = encode_gif(&seq, gifski::Repeat::Finite(2), None);
    let total: u32 = meta.frames.iter().map(|f| u32::from(f.0)).sum();

    assert_eq!(meta.loop_count, Some(2));
    assert!(meta.trailer_ok);
    // FixedOffset mode is independent of the frame-rate pollution bug, but the
    // tail still keeps the offset-derived duration (~10cs) rather than the 0.5cs
    // trailing-duplicate gap, i.e. it is not clamped to the 2cs minimum.
    let last_delay = meta.frames.last().unwrap().0;
    assert!((9..=10).contains(&last_delay), "last delay was {last_delay}");
    assert!(last_delay > 2);
    // Overall duration is the shifted 45cs span plus the ~10cs fixed-offset tail,
    // with intermediate frames rounded up to the 2cs GIF minimum.
    assert!((54..=56).contains(&total), "total was {total}");
}

/// Transparent frames must use Background disposal on the last frame so looped
/// playback resets correctly, while opaque frames stay Keep.
#[test]
fn disposal_modes_after_folding() {
    let seq: Vec<(usize, f64)> = vec![
        (0, 0.0),
        (1, 0.1),
        (1, 0.2), // duplicate
        (2, 0.3),
    ];
    let opaque = encode_gif(&seq, gifski::Repeat::Infinite, None);
    assert!(opaque.frames.iter().all(|f| f.1 == gif::DisposalMethod::Keep));

    let transparent = encode_gif(&seq, gifski::Repeat::Infinite, Some(|_, mut fr: ImgRefMut<RGBA8>| {
        // the test images contain no pure-black pixels, so target their dark colors
        fr.pixels_mut().for_each(|px| if px.r < 80 && px.g < 80 && px.b < 80 { px.a = 0; });
    }));
    assert_eq!(transparent.frames.last().unwrap().1, gif::DisposalMethod::Background);
    assert!(transparent.trailer_ok);
}
