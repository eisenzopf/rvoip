use g729::{FRAME_SAMPLES, FrameType, G729Config, G729Decoder, G729Encoder};
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};

#[cfg(feature = "itu_serial")]
#[path = "cli/itu.rs"]
mod itu;

fn usage(program: &str) {
    eprintln!("G.729AB CLI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  {program} encode [--no-vad] <input.pcm> <output.g729>");
    eprintln!("  {program} decode <input.g729> <output.pcm>");
    #[cfg(feature = "itu_serial")]
    {
        eprintln!("  {program} itu-encode <input.pcm> <output.bit> <vad_flag>");
        eprintln!("  {program} itu-decode <input.bit> <output.pcm>");
        eprintln!("  {program} test-vectors <input.bit> <expected.pcm> [annex-a|annex-b]");
    }
    eprintln!();
    eprintln!("PCM format: raw 16-bit signed little-endian mono @ 8kHz");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage(&args[0]);
        std::process::exit(2);
    }

    let code = match args[1].as_str() {
        "encode" => cmd_encode(&args),
        "decode" => cmd_decode(&args),
        #[cfg(feature = "itu_serial")]
        "itu-encode" => itu::cmd_itu_encode(&args),
        #[cfg(feature = "itu_serial")]
        "itu-decode" => itu::cmd_itu_decode(&args),
        #[cfg(feature = "itu_serial")]
        "test-vectors" => itu::cmd_test_vectors(&args),
        "help" | "--help" | "-h" => {
            usage(&args[0]);
            0
        }
        _ => {
            eprintln!("unknown command: {}", args[1]);
            usage(&args[0]);
            2
        }
    };

    std::process::exit(code);
}

fn cmd_encode(args: &[String]) -> i32 {
    let mut no_vad = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in &args[2..] {
        if arg == "--no-vad" {
            no_vad = true;
        } else {
            positional.push(arg.as_str());
        }
    }

    if positional.len() != 2 {
        eprintln!("encode expects: [--no-vad] <input.pcm> <output.g729>");
        return 2;
    }

    let input = positional[0];
    let output = positional[1];

    let mut encoder = G729Encoder::new(G729Config { annex_b: !no_vad });

    let input_file = match File::open(input) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to open {input}: {err}");
            return 1;
        }
    };
    let output_file = match File::create(output) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to create {output}: {err}");
            return 1;
        }
    };

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
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

        let mut payload = [0u8; 10];
        let frame_type = encoder.encode(&frame, &mut payload);

        let type_byte = match frame_type {
            FrameType::Speech => 1u8,
            FrameType::Sid => 2u8,
            FrameType::NoData => 0u8,
        };

        if let Err(err) = writer.write_all(&[type_byte]) {
            eprintln!("write error: {err}");
            return 1;
        }
        let write_len = frame_type.byte_len();
        if let Err(err) = writer.write_all(&payload[..write_len]) {
            eprintln!("write error: {err}");
            return 1;
        }
    }

    if let Err(err) = writer.flush() {
        eprintln!("flush error: {err}");
        return 1;
    }

    0
}

fn cmd_decode(args: &[String]) -> i32 {
    if args.len() != 4 {
        eprintln!("decode expects: <input.g729> <output.pcm>");
        return 2;
    }

    let input = &args[2];
    let output = &args[3];

    let input_file = match File::open(input) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to open {input}: {err}");
            return 1;
        }
    };
    let output_file = match File::create(output) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("failed to create {output}: {err}");
            return 1;
        }
    };

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
    let mut decoder = G729Decoder::new(G729Config::default());

    loop {
        let mut frame_tag = [0u8; 1];
        match reader.read_exact(&mut frame_tag) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => {
                eprintln!("read error: {err}");
                return 1;
            }
        }

        let mut out = [0i16; FRAME_SAMPLES];

        match frame_tag[0] {
            1 => {
                let mut speech = [0u8; 10];
                if let Err(err) = reader.read_exact(&mut speech) {
                    eprintln!("speech read error: {err}");
                    return 1;
                }
                decoder.decode_with_type(&speech, FrameType::Speech, &mut out);
            }
            2 => {
                let mut sid = [0u8; 2];
                if let Err(err) = reader.read_exact(&mut sid) {
                    eprintln!("sid read error: {err}");
                    return 1;
                }
                decoder.decode_with_type(&sid, FrameType::Sid, &mut out);
            }
            0 => {
                decoder.decode_with_type(&[], FrameType::NoData, &mut out);
            }
            other => {
                eprintln!("invalid frame tag: {other}");
                return 1;
            }
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
