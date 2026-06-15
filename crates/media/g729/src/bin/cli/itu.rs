use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use g729::bitstream::itu_serial::{
    RATE_0, RATE_8000, RATE_SID_OCTET, read_serial_frame, write_serial_frame,
};
use g729::constants::PRM_SIZE;
use g729::{FRAME_SAMPLES, FrameType, G729Config, G729Decoder, G729Encoder};

#[path = "test_vectors.rs"]
mod test_vectors;
use test_vectors::{VectorMode, run_test_vectors};

pub(crate) fn cmd_itu_encode(args: &[String]) -> i32 {
    if args.len() != 5 {
        eprintln!("itu-encode expects: <input.pcm> <output.bit> <vad_flag>");
        return 2;
    }

    let vad_flag = match args[4].parse::<i16>() {
        Ok(v) if v == 0 || v == 1 => v,
        _ => {
            eprintln!("vad_flag must be 0 or 1");
            return 2;
        }
    };

    let input_file = match File::open(&args[2]) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to open {}: {err}", args[2]);
            return 1;
        }
    };
    let output_file = match File::create(&args[3]) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to create {}: {err}", args[3]);
            return 1;
        }
    };

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
    let mut encoder = G729Encoder::new(G729Config {
        annex_b: vad_flag != 0,
    });

    let mut pcm_bytes = [0u8; FRAME_SAMPLES * 2];
    loop {
        match reader.read_exact(&mut pcm_bytes) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => {
                eprintln!("read error: {err}");
                return 1;
            }
        }

        let mut frame = [0i16; FRAME_SAMPLES];
        for i in 0..FRAME_SAMPLES {
            frame[i] = i16::from_le_bytes([pcm_bytes[2 * i], pcm_bytes[2 * i + 1]]);
        }

        let mut ana = [0i16; PRM_SIZE + 1];
        let (frame_type, _) = match encoder.encode_parm(&frame, &mut ana) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("encode_parm failed: {err}");
                return 1;
            }
        };

        let (params, rate) = match frame_type {
            FrameType::Speech => (&ana[1..1 + PRM_SIZE], RATE_8000),
            FrameType::Sid => (&ana[1..5], RATE_SID_OCTET),
            FrameType::NoData => (&ana[0..0], RATE_0),
        };

        if let Err(err) = write_serial_frame(&mut writer, params, rate) {
            eprintln!("write_serial_frame failed: {err}");
            return 1;
        }
    }

    if let Err(err) = writer.flush() {
        eprintln!("flush error: {err}");
        return 1;
    }

    0
}

pub(crate) fn cmd_itu_decode(args: &[String]) -> i32 {
    if args.len() != 4 {
        eprintln!("itu-decode expects: <input.bit> <output.pcm>");
        return 2;
    }

    let input_file = match File::open(&args[2]) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to open {}: {err}", args[2]);
            return 1;
        }
    };
    let output_file = match File::create(&args[3]) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to create {}: {err}", args[3]);
            return 1;
        }
    };

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
    let mut decoder = G729Decoder::new(G729Config::default());

    loop {
        let mut parm = [0i16; PRM_SIZE + 2 + 4];
        let mut bfi = 0i16;
        let status = match read_serial_frame(&mut reader, &mut parm, &mut bfi) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("read_serial_frame failed: {err}");
                return 1;
            }
        };
        if status == 0 {
            break;
        }

        let mut out = [0i16; FRAME_SAMPLES];
        if let Err(err) = decoder.decode_parm(&mut parm, &mut out) {
            eprintln!("decode_parm failed: {err}");
            return 1;
        }

        for sample in out {
            if let Err(err) = writer.write_all(&sample.to_le_bytes()) {
                eprintln!("write error: {err}");
                return 1;
            }
        }
    }

    if let Err(err) = writer.flush() {
        eprintln!("flush error: {err}");
        return 1;
    }

    0
}

pub(crate) fn cmd_test_vectors(args: &[String]) -> i32 {
    if args.len() < 4 || args.len() > 5 {
        eprintln!("test-vectors expects: <input.bit> <expected.pcm> [annex-a|annex-b]");
        return 2;
    }

    let mode = match VectorMode::parse(args.get(4).cloned()) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };

    match run_test_vectors(Path::new(&args[2]), Path::new(&args[3]), mode) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("FAIL: {err}");
            1
        }
    }
}
