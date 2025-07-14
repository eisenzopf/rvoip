/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#ifndef _BIT_OP_H_
#define _BIT_OP_H_

#ifndef _STDLIB_H_
#include <stdlib.h>
#endif

#include "floatutil.h"


Short GetBit( unsigned short **pBit, Short nbits );  /*defined at the end of this file */

long s_GetBitLong(
                  unsigned short **pBit,/* i/o: pointer on address of next bit */
                  Short nbits           /* i:   number of bits of code         */
                  );

void s_PushBit(Short code,Short **pBit,Short nbits);

void s_PushBitLong(
                 long code,       /* i:   codeword                       */
                 Short **pBit,    /* i/o: pointer on address of next bit */
                 Short nbits       /* i:   number of bits of code         */
                 );

#endif
