DESCRIPTION
-----------
This package includes the files needed to build the fixed point implementation
of the ITU-T G.729 Appendix IV.

DIFFERENCES WITH G.729B
-----------------------
The ANSI-C source code simulating the bit-exact, fixed point simulation 
software of enhanced VAD is integrated into the G.729B source code. 
The modification to the G.729B source code updated files of coder.c, 
cod_ld8k.c, bits.c, dec_sid.c, dtx.c, qsidgain.c, qsidlsf.c, tab_dtx.c, 
tab_dtx.h, ld8k.h, deleted vad.c/h and integrated the enhanced VAD functions
into the G.729B project. The modification also replaced the original basic 
operators used in the original G.729B source code by the basic operators of 
ITU-T Software Tool Library STL2005.

COMPILATION
-----------
For UNIX systems the following makefiles are provided

   coder.mak
   decoder.mak

Edit the makefiles coder.mak and decoder.mak to set the proper options
for your system.
The command to compile and link all code on a UNIX system is

   make -f coder.mak
   make -f decoder.mak

For other platforms, the *.mak files can be used to work out the
compilation procedures.

USAGE
-----
The command line instruction for the encoder is as follows:
   coder inputfile bitstreamfile dtx_option mode x 
where:
   inputfile    : 8 kHz sampled data file containing 16 bit linear PCM signal 
   bitstreamfile: binary file containing bitstream 
   dtx_option   : = 1 : DTX enabled   0 : DTX disabled
   mode X       : Run encoder with enhanced VAD in operating point X, 
                  X=0 or 1 or 2 representing balanced or quality-preferred 
                  or bandwidth saving operating point, respectively  

The command line for the decoder is as follows:
   decoder bitstreamfile outputfile
where:
   bitstreamfile: binary file containing bitstream
   outputfile   : 8 kHz sampled data file containing 16 bit linear PCM signal


