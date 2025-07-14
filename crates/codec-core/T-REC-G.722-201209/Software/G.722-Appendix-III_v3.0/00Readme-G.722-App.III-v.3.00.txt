/* ============================================================
ITU-T G.722 Appendix III ANSI-C Source Code

 Software Release 3.00 (2012-09) (same as 1.00, 2006-11, 
 version renumbered for consistency with G.722 3rd edition)

 Copyright (c) 2006, Broadcom Corporation

============================================================= */ 

Version:       1.0
Revision Date: Nov.02, 2006

The source code is contained in the directory named "src".  A Microsoft Visual C
6.0 workspace file is located in "workspace/VC6.0/".  Opening g722_plc_g192.dsw
will open the G.722 PLC C source code and project.

After compilation a simple test for bit-exact operation can be performed in the
directory named "testplc".  Executing the pearl script named "testplc" in the
directory will execute the test.

Calling the decg722 executable:
decg722 [-fsize N] g192_input_file speech_output_file

N: Frame size - any multiple of 160, i.e. multiple of 10 ms. 

Note that the mode and frame size is indirectly embedded in the G.192 bit-stream, 
and conflict with a command line spcified frame size will cause the decoder to use 
a frame size consistent with the G.192 bit-stream.  For frame sizes of 10 ms and 
20 ms the decoder can uniquely determine both mode and frame size from the G.192 
bit-stream.  However, for frame sizes of 30 ms and longer, a command line frame
size must be specified in order to allow the decoder to correctly determine
both mode and frame size.

--end