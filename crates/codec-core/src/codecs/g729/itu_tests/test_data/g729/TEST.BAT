rem algthm          - conditional parts of the algorithm
rem erasure         - frame erasure recovery
rem fixed           - fixed codebook search
rem lsp             - lsp quantization
rem overflow        - overflow detection in synthesizer
rem parity          - parity check
rem pitch           - pitch search
rem speech          - generic speech file
rem tame            - taming procedure

coder algthm.in algthm.bts
decoder algthm.bit algthm.pts

coder fixed.in fixed.bts
decoder fixed.bit fixed.pts

coder lsp.in lsp.bts
decoder lsp.bit lsp.pts

coder pitch.in pitch.bts
decoder pitch.bit pitch.pts

coder speech.in speech.bts
decoder speech.bit speech.pts

coder tame.in tame.bts
decoder tame.bit tame.pts

decoder erasure.bit erasure.pts
decoder overflow.bit overflow.pts
decoder parity.bit parity.pts

fc/b algthm.bit algthm.bts
fc/b algthm.pst algthm.pts

fc/b fixed.bit fixed.bts
fc/b fixed.pst fixed.pts

fc/b lsp.bit lsp.bts
fc/b lsp.pst lsp.pts

fc/b pitch.bit pitch.bts
fc/b pitch.pst pitch.pts

fc/b speech.bit speech.bts
fc/b speech.pst speech.pts

fc/b tame.bit tame.bts
fc/b tame.pst tame.pts

fc/b erasure.pst erasure.pts
fc/b overflow.pst overflow.pts
fc/b parity.pst parity.pts

