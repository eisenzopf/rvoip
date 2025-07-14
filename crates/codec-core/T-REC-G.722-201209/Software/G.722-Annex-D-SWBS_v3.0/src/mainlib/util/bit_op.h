/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#ifndef _BIT_OP_H_
#define _BIT_OP_H_

#ifndef _STDLIB_H_
#include <stdlib.h>
#endif
#ifndef _STL_H
#include "stl.h"
#endif

Word16 GetBit( UWord16 **pBit, Word16 nbits );  /*defined at the end of this file */

Word16 GetBit1( UWord16 **pBit);  /*defined at the end of this file */

Word32 GetBitLong(
                  UWord16 **pBit,     /* i/o: pointer on address of next bit */
                  Word16 nbits        /* i:   number of bits of code         */
                  );

void PushBit(Word16 code, UWord16 **pBit, Word16 nbits);

void PushBit1(Word16 code, UWord16 **pBit);

void PushBitLong(
                 Word32 code,       /* i:   codeword                       */
                 UWord16 **pBit,    /* i/o: pointer on address of next bit */
                 Word16 nbits       /* i:   number of bits of code         */
                 );

#endif
