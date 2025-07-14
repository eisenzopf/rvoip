/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/
#ifndef G722_COM_H
#define G722_COM_H 200

/* Include function prototypes for G.192 bitstreams*/
#include "softbit.h"

#define MAX_STR 1024                            /* file names */
#define DEF_FR_SIZE      80                    /* 16kHz, 5 ms input frame size  */
#define MAX_INPUT_SP_BUFFER 8190                /* maximum input 16 khz speech frame size, every 16 bit 16 khz input sample yields an 8 bits at 8 kHz */
#define MAX_OUTPUT_SP_BUFFER MAX_INPUT_SP_BUFFER                    

#define MAX_BITLEN_SIZE (8*MAX_INPUT_SP_BUFFER/2)       /* G.192 framelen maximum limit in bits, (frame length is read as a short value<=32767) */	
#define MAX_BYTESTREAM_BUFFER (MAX_INPUT_SP_BUFFER/2)   /* non g192 coded data each 16 bit input sample 16 kHz is stored in one byte (8 bits at 8 KHz) */	

#define MAX_PLC         3        /* max plc algorithm number*/ 
#define PLC_MEM_SIZE    4        /* number of frames with PLC index memory */
#define N_MODES         3        /* 1,2,3 (g722 modes) */
#define ZERO_INDEX      0x00FF   /* index for zeroing */



/* ................. End of file g722_com.h .................................. */
#endif
