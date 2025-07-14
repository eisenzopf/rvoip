====================================================================================
 ITU-T G.722 PLC Appendix IV

 Software Release 3.00 (2012-09) (same as 1.2, 2009-11-06, 
 version renumbered for consistency with G.722 3rd edition)

====================================================================================

These files represent the ITU-T G.722 Appendix IV Bit-Exact C
simulation. The version 1.2 reflects the approval of revised Appendix
IV on 6 Nov 2009, with a change in file g722.c. All other files remain
the same, except for the code version number.

All code is written in ANSI-C.

The packet loss concealment (PLC) algorithm is implemented as part of the decoder.

   decg722 [options] g192_bst output

NOTE: The folder "testvectors" contains a set of test vectors to verify the 
proper compilation of the reference software. Please refer to the file 
"00readme_tv.txt" in that folder.

                            FILE FORMATS:
                            =============

The file format of the supplied binary data is 16-bit binary data which is
read and written in 16 bit little-endian words.  
The data is therefore platform DEPENDENT.  

The bitstream follows the ITU-T G.192 format. For every 20 ms input speech frame,
the bitstream contains the following data:

	Word16 SyncWord
	Word16 DataLen
	Word16 1st Databit
	Word16 2nd DataBit
	.
	.
	.
	Word16 Nth DataBit

Each bit is presented as follows: Bit 0 = 0x007f, Bit 1 = 0x0081.

The SyncWord from the encoder is always 0x6b21. The SyncWord 0x6b20, on decoder side, 
indicates that the current frame was received in error (bad frame).

The DataLen parameter gives the number of speech data bits in the frame. 

			INSTALLING THE SOFTWARE
			=======================

The package includes makefile for gcc and workspace for Visual C++ 6.0. 
The makefile can be used as follows:

   make -f Makefile.gcc

assuming you have gcc installed.

The G.722 App. IV algorithm has been successfully compiled on Windows using Visual C++ 6.0
and gcc/Cygwin and on Linux using gcc.

                       RUNNING THE SOFTWARE
                       ====================

The command line for the G.722 decoder with PLC is as follows:

   decg722 [-fsize N] g192_bst output

   where N is frame size at 16 kHz (default: 160)

The output file is a sampled data file containing 16 bit PCM signals.
The mapping table of the encoded bitstream is contained in the simulation software.






