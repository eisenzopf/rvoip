/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "bit_op.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-----------------------------------------------------------------*
*   Funtion  GetBit                                               *
*            ~~~~~~~~~~~~                                         *
*   Read indice from the bitstream.                               *
*-----------------------------------------------------------------*/
/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
Word16 GetBit(
              UWord16 **pBit, /* i/o: pointer on address of next bit */
              Word16 nbits    /* i:   number of bits of code         */
              )
{
  Word16 i, code, temp16;
#ifdef DYN_RAM_CNT
  {
    UWord32	ssize;
    ssize = (UWord32) (3*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  code = 0;	move16();	
  FOR( i = 0; i < nbits; i++ )	
  {
    code = shl(code, 1);

    temp16 = (Word16)1;		move16(); 
    if (sub(**pBit, 0x007f) == 0)
    {
      temp16 = 0;		move16(); 
    }
    code = s_or( code, temp16);

    (*pBit)++;		  
  }

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/

  return( code );
}


/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
Word32 GetBitLong(
                  UWord16 **pBit,     /* i/o: pointer on address of next bit */
                  Word16 nbits        /* i:   number of bits of code         */
                  )
{
  Word16 i;
  Word32 code, temp16;

/*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) (1 * SIZE_Word16);
    ssize += (UWord32) (2 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
/*****************************/

  code = 0;
  move32();
  FOR( i = 0; i < nbits; i++ )	
  {
    code = L_shl(code,1);
    temp16 = (Word32) 1;	move32();	
    if ( sub(**pBit, 0x007f) == 0)
    {
      temp16 = 0;
      move16();
    }
    code = L_or( code, temp16);

    (*pBit)++;			
  }

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/

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
void PushBit(
             Word16 code,    /* i:   codeword                       */
             UWord16 **pBit, /* i/o: pointer on address of next bit */
             Word16 nbits    /* i:   number of bits of code         */
             )
{
  Word16 i, nbitm1, mask;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16), "dummy");
#endif

  /* MSB -> LSB */

  nbitm1 = sub( nbits, 1);

  FOR( i = nbitm1; i >= 0; i-- )	
  {
    *(*pBit) = 0x0081;		move16();	
    mask = s_and(shr(code, i), 0x0001);

    if (mask == 0)
    {
      *(*pBit) = 0x007f;	move16();	
    }

    (*pBit)++;
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
}

/*
* BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
*/
void PushBitLong(
                 Word32 code,       /* i:   codeword                       */
                 UWord16 **pBit,    /* i/o: pointer on address of next bit */
                 Word16 nbits       /* i:   number of bits of code         */
                 )
{
  Word16 i, nbitm1;
  Word32 mask;

/*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) (2 * SIZE_Word16);
    ssize += (UWord32) (1 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
/*****************************/

  /* MSB -> LSB */
  nbitm1 = sub(nbits, 1);
  FOR( i = nbitm1; i >= 0; i-- )	
  {
    *(*pBit) = 0x0081;
    mask = L_and( L_shr(code,i), 0x0001);

    if (mask == 0)
    {
      *(*pBit) = 0x007f;
    }

    (*pBit)++;
  }

/*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
/*****************************/
}

