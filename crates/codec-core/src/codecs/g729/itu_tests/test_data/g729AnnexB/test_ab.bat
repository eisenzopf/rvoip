echo "Verification of G.729A with Annex B" > g729ab.log
..\c-codeBA\coder tstseq1.bin tstseq1a.bit.tst 1
fc/b tstseq1a.bit.tst tstseq1a.bit >> g729ab.log
..\c-codeBA\coder tstseq2.bin tstseq2a.bit.tst 1
fc/b tstseq2a.bit.tst tstseq2a.bit >> g729ab.log
..\c-codeBA\coder tstseq3.bin tstseq3a.bit.tst 1
fc/b tstseq3a.bit.tst tstseq3a.bit >> g729ab.log
..\c-codeBA\coder tstseq4.bin tstseq4a.bit.tst 1
fc/b tstseq4a.bit.tst tstseq4a.bit >> g729ab.log

..\c-codeBA\decoder tstseq1a.bit tstseq1a.out.tst
fc/b tstseq1a.out.tst tstseq1a.out >> g729ab.log
..\c-codeBA\decoder tstseq2a.bit tstseq2a.out.tst
fc/b tstseq2a.out.tst tstseq2a.out >> g729ab.log
..\c-codeBA\decoder tstseq3a.bit tstseq3a.out.tst
fc/b tstseq3a.out.tst tstseq3a.out >> g729ab.log
..\c-codeBA\decoder tstseq4a.bit tstseq4a.out.tst
fc/b tstseq4a.out.tst tstseq4a.out >> g729ab.log
..\c-codeBA\decoder tstseq5.bit tstseq5a.out.tst
fc/b tstseq5a.out.tst tstseq5a.out >> g729ab.log
..\c-codeBA\decoder tstseq6.bit tstseq6a.out.tst
fc/b tstseq6a.out.tst tstseq6a.out >> g729ab.log
