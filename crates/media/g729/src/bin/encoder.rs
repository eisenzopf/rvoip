use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};

use g729::bitstream::itu_serial::{RATE_0, RATE_8000, RATE_SID_OCTET, write_serial_frame};
use g729::constants::PRM_SIZE;
use g729::{FRAME_SAMPLES, FrameType, G729Config, G729Encoder};

fn main() {
    let mut args = std::env::args();
    let _prog = args.next();

    let input = match args.next() {
        Some(v) => v,
        None => {
            eprintln!("usage: encoder <input.pcm> <output.bit> [vad_flag]");
            std::process::exit(2);
        }
    };
    let output = match args.next() {
        Some(v) => v,
        None => {
            eprintln!("usage: encoder <input.pcm> <output.bit> [vad_flag]");
            std::process::exit(2);
        }
    };
    let vad_flag = args.next().unwrap_or_else(|| "0".to_string());

    let mut encoder = G729Encoder::new(G729Config {
        annex_b: vad_flag == "1",
    });

    let input_file = File::open(&input).unwrap_or_else(|e| {
        eprintln!("failed to read PCM input: {e}");
        std::process::exit(1);
    });
    let output_file = File::create(&output).unwrap_or_else(|e| {
        eprintln!("failed to write bitstream output: {e}");
        std::process::exit(1);
    });

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
    let mut pcm_bytes = [0u8; FRAME_SAMPLES * 2];

    loop {
        match reader.read_exact(&mut pcm_bytes) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => {
                eprintln!("read error: {err}");
                std::process::exit(1);
            }
        }

        let mut frame = [0i16; FRAME_SAMPLES];
        for i in 0..FRAME_SAMPLES {
            frame[i] = i16::from_le_bytes([pcm_bytes[2 * i], pcm_bytes[2 * i + 1]]);
        }

        let mut ana = [0i16; PRM_SIZE + 1];
        let (frame_type, _) = encoder.encode_parm(&frame, &mut ana).unwrap_or_else(|e| {
            eprintln!("encode_parm failed: {e}");
            std::process::exit(1);
        });

        let (params, rate) = match frame_type {
            FrameType::Speech => (&ana[1..1 + PRM_SIZE], RATE_8000),
            FrameType::Sid => (&ana[1..5], RATE_SID_OCTET),
            FrameType::NoData => (&ana[0..0], RATE_0),
        };

        if let Err(err) = write_serial_frame(&mut writer, params, rate) {
            eprintln!("write_serial_frame failed: {err}");
            std::process::exit(1);
        }
    }

    writer.flush().unwrap_or_else(|e| {
        eprintln!("flush failed: {e}");
        std::process::exit(1);
    });
}
