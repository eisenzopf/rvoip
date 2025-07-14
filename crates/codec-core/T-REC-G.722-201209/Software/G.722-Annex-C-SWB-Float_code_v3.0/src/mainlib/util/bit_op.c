/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "bit_op.h"


/*-----------------------------------------------------------------*
*   Funtion  GetBit                                               *
*            ~~~~~~~~~~~~                                         *
*   Read indice from the bitstream.                               *
*-----------------------------------------------------------------*/
/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
Short GetBit(
              unsigned short **pBit, /* i/o: pointer on address of next bit */
              Short nbits    /* i:   number of bits of code         */
              )
{
  Short i, code, temp16;

  code = 0;
  for( i = 0; i < nbits; i++ )	
  {
    code = code << 1;

    temp16 = (Short)1;
    if( **pBit == 0x007f )
    {
      temp16 = 0;
    }
    code = code | temp16;

    (*pBit)++;		  
  }
  return( code );
}



/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
long s_GetBitLong(
                  unsigned short **pBit, /* i/o: pointer on address of next bit */
                  Short nbits            /* i:   number of bits of code         */
                  )
{
  Short i;
  long code, temp16;

  code = 0;
  for( i = 0; i < nbits; i++ )	
  {
    code = code << 1;
    temp16 = 1L;
    if (**pBit == 0x007f)
    {
      temp16 = 0;
    }
    code = code | temp16;

    (*pBit)++;			
  }

  return( code );
}


/*-----------------------------------------------------------------*
*   Funtion  PushBit                                              *
*            ~~~~~~~~~~~~                                         *
*   Write indice to the bitstream.                                *
*-----------------------------------------------------------------*/
/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
void s_PushBit(
             Short code,    /* i:   codeword                       */
             Short **pBit, /* i/o: pointer on address of next bit */
             Short nbits    /* i:   number of bits of code         */
             )
{
  Short i, nbitm1, mask;

  /* MSB -> LSB */

  nbitm1 = nbits - 1;

  for( i = nbitm1; i >= 0; i-- )	
  {
    *(*pBit) = 0x0081;
    mask = (code>>i) & 0x0001;

    if( mask == 0 )
    {
      *(*pBit) = 0x007f;
    }

    (*pBit)++;
  }
}



/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
void s_PushBitLong(
                 long code,       /* i:   codeword                       */
                 Short **pBit,    /* i/o: pointer on address of next bit */
                 Short nbits       /* i:   number of bits of code         */
                 )
{
  Short i, nbitm1;
  long mask;

  /* MSB -> LSB */
  nbitm1 = nbits - 1;
  for( i = nbitm1; i >= 0; i-- )	
  {
    *(*pBit) = 0x0081;
    mask = (code>>i) & 0x0001;

    if( mask == 0 )
    {
      *(*pBit) = 0x007f;
    }

    (*pBit)++;
  }
}
