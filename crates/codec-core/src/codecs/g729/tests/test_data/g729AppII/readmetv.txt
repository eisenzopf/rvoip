This group of test vectors are for use with G.729 Annex A + G.729 Annex B Appendix II

Usage:

Input File                     Processing       Output File

tstseq1.bin                    encoder          tstseq1a_appendixII.bit
tstseq2.bin                    encoder          tstseq2a_appendixII.bit
tstseq3.bin                    encoder          tstseq3a_appendixII.bit
tstseq4.bin                    encoder          tstseq4a_appendixII.bit
tstseq1a_appendixII.bit        decoder          tstseq1a_appendixII.out
tstseq2a_appendixII.bit        decoder          tstseq2a_appendixII.out
tstseq3a_appendixII.bit        decoder          tstseq3a_appendixII.out
tstseq4a_appendixII.bit        decoder          tstseq4a_appendixII.out
tstseq5.bit                    decoder          tstseq5a.out
tstseq6.bit                    decoder          tstseq6a.out
tstseq7.bin                    encoder          tstseq7a_appendixII.bit
tstseq7a_appendixII.bit        decoder          tstseq7a_appendixII.out

tstseq7.bin                    encoder*         tstseq7a.bit
tstseq7a.bit                   decoder*         tstseq7a.out          

Note : tstseq7.bin is a 440Hz tone. 
It is added to Appendix II test vectors to show how Appendix II behaves with tones.

* For information and comparison, outputs of G.729 Annex A + G.729 Annex B coder and decoder are provided to show tones problem.

(C) ITU 2005