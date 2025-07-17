 ITU-T G.729 Software Package Release 2 (November 2006) 
Testvectors to verify correct execution of G.729 ANSI-C software
Version 3.3

This directory contains testvectors to validate the correct execution
of the G.729 ANSI-C software (version 3.3). NOTE that these vectors
are not part of a validation procedure. It is very difficult to design
an exhaustive set of test vectors. Hence passing these vectors should
be viewed as a minimum requirement, and is not a guarantee that the
implementation is correct for every possible input signal.

Format: all files contain 16 bit sampled data using the Intel (PC)
format.

*.in  - input files
*.bit - bit stream files
*.out - output files

and were obtained using the following commands

 coder file.in file.bit
 decoder file.bit file.pst

Note that for some files only the *.bit and *.pst are available.

The testvectors were designed to provide as much coverage as possible
in terms of parameters and algorithm. Below we indicate what parts of
the algorithms are excercised. Note that none of these sequences
provides an exhaustive coverage.

algthm		- conditional parts of the algorithm 
erasure		- frame erasure recovery
fixed		- fixed codebook search
lsp		- lsp quantization
overflow	- overflow detection in synthesizer
parity		- parity check
pitch		- pitch search
speech		- generic speech file
tame		- taming procedure

List of files and size in bytes

    5740  algthm.bit
    5600  algthm.in
    5600  algthm.pst
   49200  erasure.bit
   48000  erasure.pst
   19680  fixed.bit
   19200  fixed.in
   19200  fixed.pst
  366048  lsp.bit
  357120  lsp.in
  357120  lsp.pst
   62976  overflow.bit
   61440  overflow.pst
   49200  parity.bit
   48000  parity.pst
  300940  pitch.bit
  293628  pitch.in
  293600  pitch.pst
  615000  speech.bit
  600064  speech.in
  600000  speech.pst
   20992  tame.bit
   20480  tame.in
   20480  tame.pst
