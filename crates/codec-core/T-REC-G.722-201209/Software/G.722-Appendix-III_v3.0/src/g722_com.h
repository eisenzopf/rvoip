/* ITU G.722 3rd Edition (2012-09) */

/*
  ============================================================================
   File: G722_COM.H                                  v3.0 beta - 23/Aug/2006                    
  ============================================================================

                            UGST/ITU-T G722 MODULE

                          GLOBAL COMMON DEFINES

   History:
    first version  22 may 2006

  ============================================================================
*/
#ifndef G722_COM_H
#define G722_COM_H 200

/* Include function prototypes for G192 bitstreams*/
#include "softbit.h"

#define MAX_STR 1024                            /* file names */
#define MAX_INPUT_SP_BUFFER 8190                /* maximum input 16 khz speech frame size, every 16 bit 16 khz input sample yields an 8 bits at 8 kHz */
#define MAX_OUTPUT_SP_BUFFER MAX_INPUT_SP_BUFFER                    

#define MAX_BITLEN_SIZE (8*MAX_INPUT_SP_BUFFER/2)       /* G.192 framelen maximum limit in bits, (frame length is read as a short value<=32767) */
#define MAX_BYTESTREAM_BUFFER (MAX_INPUT_SP_BUFFER/2)   /* non g192 coded data each 16 bit input sample 16 kHz is stored in one byte (8 bits at 8 KHz) */

#define N_MODES         3        /* 1,2,3 (g722 modes) */



/* ................. End of file g722_com.h .................................. */
#endif
