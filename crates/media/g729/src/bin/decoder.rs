use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use g729::bitstream::itu_serial::read_serial_frame;
use g729::constants::PRM_SIZE;
use g729::{FRAME_SAMPLES, G729Config, G729Decoder};

fn main() {
    let mut args = std::env::args();
    let _prog = args.next();

    let input = match args.next() {
        Some(v) => v,
        None => {
            eprintln!("usage: decoder <input.bit> <output.pcm>");
            std::process::exit(2);
        }
    };
    let output = match args.next() {
        Some(v) => v,
        None => {
            eprintln!("usage: decoder <input.bit> <output.pcm>");
            std::process::exit(2);
        }
    };

    let input_file = File::open(&input).unwrap_or_else(|e| {
        eprintln!("failed to read bitstream input: {e}");
        std::process::exit(1);
    });
    let output_file = File::create(&output).unwrap_or_else(|e| {
        eprintln!("failed to write PCM output: {e}");
        std::process::exit(1);
    });

    let mut reader = BufReader::new(input_file);
    let mut writer = BufWriter::new(output_file);
    let mut decoder = G729Decoder::new(G729Config::default());

    loop {
        let mut parm = [0i16; PRM_SIZE + 2 + 4];
        let mut bfi = 0i16;

        let status = read_serial_frame(&mut reader, &mut parm, &mut bfi).unwrap_or_else(|e| {
            eprintln!("read_serial_frame failed: {e}");
            std::process::exit(1);
        });
        if status == 0 {
            break;
        }

        let mut out = [0i16; FRAME_SAMPLES];
        decoder
            .decode_parm(&mut parm, &mut out)
            .unwrap_or_else(|e| {
                eprintln!("decode_parm failed: {e}");
                std::process::exit(1);
            });

        for sample in out {
            writer.write_all(&sample.to_le_bytes()).unwrap_or_else(|e| {
                eprintln!("write failed: {e}");
                std::process::exit(1);
            });
        }
    }

    writer.flush().unwrap_or_else(|e| {
        eprintln!("flush failed: {e}");
        std::process::exit(1);
    });
}
