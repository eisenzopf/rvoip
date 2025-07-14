/*--------------------------------------------------------------------------
 ITU-T G.722 Annex B (ex G.722-SWB) Source Code

 Software Release 3.00 (2012-09) (same as 1.00, 2010-09, 
 version renumbered for consistency with G.722 3rd edition)

 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

These files represent the ITU-T G.722-SWB Coder Fixed-Point Bit-Exact C simulation.
All code is written in ANSI-C.  The coder is implemented as two separate programs:

        encoder [-options] <infile> <codefile> [rate]
        decoder [-options] <codefile> <outfile> [rate] 
	
The folder "test_vector" contains a set of test vectors to verify the 
proper compilation of the reference software.


                            FILE FORMATS:
                            =============

The file format of the supplied binary data is 16-bit binary data which is
read and written in 16 bit little-endian words.
The data is therefore platform DEPENDENT.

The bitstream follows the ITU-T G.192 format. For every 5-ms input speech frame,
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
indicates that the current frame was received in error (frame erasure).

The DataLen parameter gives the number of speech data bits in the frame. 


			INSTALLING THE SOFTWARE
			=======================

Installing the software on the PC:

The package includes Makefile for gcc and makefile.cl for Visual C++. 
The makefiles can be used as follows:

Linux/Cygwin: make
Visual C++  : nmake -f makefile.cl

The codec has been successfully compiled on Linux/Cygwin using gcc
and Windows using Visual C++.

NOTE: Visual C++ command prompt should be used to compile with Visual C++ on Windows.


                       RUNNING THE SOFTWARE
                       ====================

The command line for the encoder is as follows:

  encoder [-options] <infile> <codefile> [rate]

  where:
    rate       is the desired encoding bitrate in kbit/s:
               64 for G.722 core at 56 kbit/s and
               96 for G.722 core at 64 kbit/s
    infile     is the name of the input file to be encoded
    codefile   is the name of the output bitstream file

  Options:
    -quiet     quiet processing

The command line for the decoder is as follows:

  decoder [-options] <codefile> <outfile> [rate]

  where:
    rate       is the desired decoding bitrate in kbit/s:
               64 for G.722 R1sm,
               80 for G.722 R2sm and
               96 for G.722 R3sm
    codefile   is the name of the input bitstream file
    outfile    is the name of the decoded output file

  Options:	
    -quiet     quiet processing
    -bitrateswitch [bsflag]
               bsflag is 1 for G.722 core at 56 kbit/s
                     and 0 for G.722 core at 64 kbit/s

If you run the software on Windows, you can make a set of test vectors using:

Cygwin    : make proc
Visual C++: nmake -f makefile.cl proc

or

test proc

easily. The test vectors are stored in folder "out".

NOTE: This batch file is not available on non-Windows platforms, because the tools
in folder "tools" are Windows executable files.
