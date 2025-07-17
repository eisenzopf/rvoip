# G.729 ITU Test Data

This directory contains all the ITU-T G.729 test vectors and reference data from the G.729 Release 3 package.

## Directory Structure

### Test Vectors Available

- **g729/**: Base G.729 test vectors (25 files including speech, pitch, LSP, algorithm tests)
- **g729AnnexA/**: G.729 Annex A reduced complexity test vectors (28 files)
- **g729AnnexB/**: G.729 Annex B VAD/DTX/CNG test vectors (29 files)
- **g729AnnexD/**: G.729 Annex D V.8 bis compatibility test vectors
- **g729AnnexE/**: G.729 Annex E 11.8 kbit/s extension test vectors (30 files)
- **g729AnnexF/**: G.729 Annex F 6.4 kbit/s with DTX test vectors
- **g729AnnexG/**: G.729 Annex G dual rate with DTX test vectors
- **g729AnnexH/**: G.729 Annex H test vectors
- **g729AnnexI/**: G.729 Annex I fixed-point implementation test vectors
- **g729AppII/**: G.729 Application II wideband extension test vectors
- **g729AppIII/**: G.729 Application III floating to fixed-point test vectors

### Source Code Only

- **g729AnnexC/**: No test vectors (contains only c_code)
- **g729AnnexC+/**: No test vectors (contains only c_code)
- **g729AppIV/**: Complete source code for enhanced VAD implementation (74 files)

## Test Vector Types

The test vectors include:

### Input Files (.IN, .bin)
- Speech samples for encoding tests
- Various signal types (speech, algorithm, pitch, LSP tests)

### Bitstream Files (.BIT, .bit)
- Encoded bitstreams for decoder validation
- Error condition tests (parity, erasure, overflow)

### Output Files (.PST, .out)
- Reference decoder outputs for comparison
- Post-filtered and processed speech samples

### Special Test Cases
- **SPEECH**: Natural speech samples
- **ALGTHM**: Algorithm validation signals
- **PITCH**: Pitch analysis test signals
- **LSP**: Line Spectral Pair quantization tests
- **FIXED**: Fixed codebook search tests
- **TAME**: Taming procedure tests
- **PARITY**: Parity error handling tests
- **ERASURE**: Frame erasure handling tests
- **OVERFLOW**: Overflow detection tests

## Total Test Files

- **49 bitstream files** (.BIT/.bit) across all variants
- Hundreds of input/output reference files
- Complete ITU compliance test suite

## Usage

These test vectors should be used to validate:

1. **Encoder compliance**: Encode .IN files and compare bitstreams with .BIT files
2. **Decoder compliance**: Decode .BIT files and compare outputs with .PST/.out files
3. **Error handling**: Test with parity, erasure, and overflow test cases
4. **Quality validation**: Subjective and objective quality assessment

## Notes

- All test vectors are from ITU-T G.729 Release 3 (November 2006)
- Test vectors maintain original ITU naming conventions
- Binary files are in the original format from the ITU package
- Some variants (AnnexC, AnnexC+) provide only source code, not test vectors 