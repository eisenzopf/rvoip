README FILE FOR G.729 ANNEX E TEST VECTORS
------------------------------------------

This is the readme for the test vectors to be used to verify the
correct execution of G.729 Annex E ANSI-C software Version 1.3

This directory contains test vectors to validate the correct execution
of the G.729 Annex E (ANSI-C software version 1.3). NOTE that these
vectors are not part of a validation procedure. It is very difficult
to design an exhaustive set of test vectors. Hence passing these
vectors should be viewed as a minimum requirement, and is not a
guarantee that the implementation is correct for every possible input
signal.

The test vectors were designed to provide as much coverage as possible
in terms of parameters, states and algorithm. Below we indicate what
parts of the algorithms are tested. Note that none of these sequences
provides an exhaustive coverage.

----------------------------------------------------------------------
NOTE:
----------------------------------------------------------------------
The test vectors for the decoder output in version 1.3 differ from
those previously available for V.1.2. However, the encoder input and
output test vectors, as well as the decoder _input_ test vectors are
the same as for V.1.2.
----------------------------------------------------------------------

*** CONTENTS

algthme     - conditional parts of the algorithm 
erasuree    - frame erasure recovery
fixed35     - 35-bit fixed codebook search
fixed44     - 44-bit fixed codebook search
lspe        - lsp quantization
ovpitch     - overflow condition in the calculation of the pitch gain 
pitche      - pitch search
speeche     - generic speech file
stabilit    - LSP stability
switch      - bitrate switching 

*.118 and .xxx  - input files
*.bit           - bit stream files
*.pst           - output files
switch          - rate switching command

Note that for some files only the *.bit and *.pst are available.

List of files and size in bytes

    4800  algthme.118
    7200  algthme.bit
    4800  algthme.pst
   11280  erasuree.bit
    7520  erasuree.pst
   81120  fixed35.118
  121680  fixed35.bit
   81120  fixed35.pst
  228800  fixed44.118
  343200  fixed44.bit
  228800  fixed44.pst
  204800  lspe.118
  307200  lspe.bit
  204800  lspe.pst
   80000  ovpitch.118
  120000  ovpitch.bit
   80000  ovpitch.pst
  236640  pitche.118
  354960  pitche.bit
  236640  pitche.pst
  600064  speeche.118
  900000  speeche.bit
  600000  speeche.pst
   37680  stabilit.bit
   25120  stabilit.pst
    1772  switch
  182240  switch.bit
  141760  switch.pst
  141824  switch.xxx


*** FORMAT

all files contain 16 bit little-endian data


*** GENERATION COMMAND (see test.bat)

The test vectors were obtained using the following commands:

 codere file.118 file.bit 1
 decodere file.bit file.pst

A special syntax was used for the rate switching test :

  codere switch.xxx switch.bit switch
  decodere switch.bit switch.pst

[END]