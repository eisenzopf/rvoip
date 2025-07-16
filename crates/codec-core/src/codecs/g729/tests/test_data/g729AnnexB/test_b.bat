echo "Verification of G.729 Annex B" > g729b.log
..\c-codeB\coder tstseq1.bin tstseq1.bit.tst 1
fc/b tstseq1.bit.tst tstseq1.bit >> g729b.log
..\c-codeB\coder tstseq2.bin tstseq2.bit.tst 1
fc/b tstseq2.bit.tst tstseq2.bit >> g729b.log
..\c-codeB\coder tstseq3.bin tstseq3.bit.tst 1
fc/b tstseq3.bit.tst tstseq3.bit >> g729b.log
..\c-codeB\coder tstseq4.bin tstseq4.bit.tst 1
fc/b tstseq4.bit.tst tstseq4.bit >> g729b.log

..\c-codeB\decoder tstseq1.bit tstseq1.out.tst
fc/b tstseq1.out.tst tstseq1.out >> g729b.log
..\c-codeB\decoder tstseq2.bit tstseq2.out.tst
fc/b tstseq2.out.tst tstseq2.out >> g729b.log
..\c-codeB\decoder tstseq3.bit tstseq3.out.tst
fc/b tstseq3.out.tst tstseq3.out >> g729b.log
..\c-codeB\decoder tstseq4.bit tstseq4.out.tst
fc/b tstseq4.out.tst tstseq4.out >> g729b.log
..\c-codeB\decoder tstseq5.bit tstseq5.out.tst
fc/b tstseq5.out.tst tstseq5.out >> g729b.log
..\c-codeB\decoder tstseq6.bit tstseq6.out.tst
fc/b tstseq6.out.tst tstseq6.out >> g729b.log
