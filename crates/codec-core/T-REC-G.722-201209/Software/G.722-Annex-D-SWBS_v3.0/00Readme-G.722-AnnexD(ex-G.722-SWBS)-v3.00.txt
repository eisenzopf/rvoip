/*--------------------------------------------------------------------------------------
 ITU-T G.722 Annex D (ex G.722-SWBS) - Reference C code for fixed-point implementation          

 Software Release 3.00 (2012-09) (same as 1.00, version renumbered for 
 consistency with G.722 3rd edition)

 Copyright (c) 2012, Huawei Technologies, France Telecom
--------------------------------------------------------------------------------------*/

These files represent the ITU-T G.722-SWBS Coder Fixed-Point Bit-Exact C simulation.
All code is written in ANSI-C.  The coder is implemented as two separate programs:

        encoder -stereo [-options] <infile> <codefile> [rate]
        decoder [-options] <codefile> <outfile> [rate] 
	
The folder "G.722-SWBS_20120614_TV" contains a set of test vectors to verify the 
proper compilation of the reference software.

WARNING: These test vectors are provided to verify correct execution of
the software on the target platform. They cannot be used to verify
compliance to the standard.


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

with N = DataLen
Each bit is presented as follows: Bit 0 = 0x007f, Bit 1 = 0x0081.

The SyncWord from the encoder is always 0x6b21. The SyncWord 0x6b20, on decoder side, 
indicates that the current frame was received in error (frame erasure).

The DataLen parameter gives the number of speech data bits in the frame. 


			INSTALLING THE SOFTWARE
			=======================

Installing the software on the PC:

The package includes a Visual C++ 2008 solution. 

This code has been successfully compiled and run on the following
platforms:
 
Platform                   Operating System      Compiler
-----------------------------------------------------------------------------
PC                         Windows 7             Visual C++ 2008 Express Edition


                       RUNNING THE SOFTWARE
                       ====================
The command line for the WB stereo encoder is as follows:

encoder -stereo -wb [-options] <infile> <codefile> [rate]

where
        rate            is the desired encoding bitrate in kbit/s: either 64 or 80 (64 for R1ws or 80 for R2ws)
        infile          is the name of the input file to be encoded
        codefile        is the name of the output bitstream file
Options:
        -quiet	        quiet processing
        
        
The command line for the SWB stereo encoder is as follows:

encoder -stereo [-options] <infile> <codefile> [rate]

where
        rate            is the desired encoding bitrate in kbit/s: (80 for R2ss, 96 for R3ss,112 for R4ss and 128 for R5ss )
        infile          is the name of the input file to be encoded
        codefile        is the name of the output bitstream file
Options:
        -quiet          quiet processing


The command line for the WB and SWB stereo decoder are the same, as follows:

decoder [-options] <codefile> <outfile> [rate]

where
        rate            is the desired decoding bitrate in kbit/s: 64 R1ws, 80 for R2ws or R2ss, 96 for R3ss,112 for R4ss and 128 for R5ss
        codefile        is the name of the input bitstream file
        outfile         is the name of the decoded output file
Options:	
        -quiet          quiet processing
        -bitrateswitch [bsflag]        bsflag is 1 for G.722 core at 56kbit/s and 0 for G.722 core at 64 kbit/s

The encoder input and the decoder output files are sampled data files containing 
16-bit PCM signals. The encoder output and decoder input files follow the ITU-T 
G.192 bitstream format.

