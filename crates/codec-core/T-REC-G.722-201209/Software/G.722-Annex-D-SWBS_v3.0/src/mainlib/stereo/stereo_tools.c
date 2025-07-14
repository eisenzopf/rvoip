/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

#ifdef LAYER_STEREO
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "stereo_tools.h"
#include "pcmswb.h"
#include "softbit.h"
#include "g722_stereo.h"
#include "oper_32b.h"
#include "control.h"
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif

/*************************************************************************
* deinterleave
*
* Splits an interleaved stereo signal of length N into two separated 
* signals of length N/2
**************************************************************************/
void deinterleave(const Word16* input, /* i: Interleaved input signal */
                        Word16* left,  /* o: Left output channel */
                        Word16* right, /* o: Right output channel */
                        Word16  N      /* Number of samples in input frame */
                        )
{
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR(i = 0; i < N/2; i++)
    {
        left[i]  = input[2*i];  move16();
        right[i] = input[2*i+1];move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* interleave
*
* Creates a stereo interleaved signal of length 2*N from two separated 
* signals of length N
**************************************************************************/
void interleave(Word16* left,   /* i: Left input channel */
                Word16* right,  /* i: Right input channel */
                Word16* output, /* o: Interleaved output signal */
                Word16  N       /* Number of samples in input frames */
                )
{
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR(i = N-1; i >= 0; i--)
    {
        output[2*i+1] = right[i];move16();
        output[2*i]   = left[i]; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* OLA
*
* overlap and add for mono signal
**************************************************************************/
void OLA(Word16 *cur_real, /* i: current frame */
         Word16 *mem_real, /* i: past frame */
         Word16 *cur_ola   /* o: ouptut overlap and add */
         )
{
    Word16 i, tmp;
    Word16 *ptr0, *ptr0b, *ptr1;
    const Word16 *ptrwin;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    ptrwin = win_D;
    ptr0   = cur_real;
    ptr0b  = mem_real;
    ptr1   = cur_ola;

    FOR(i=0; i< 58 ; i++)
    {
        tmp = mult(*ptr0++, *ptrwin++); 
        *ptr1++ = add(*ptr0b++, tmp); move16();
    }
    mov16(22, ptr0, ptr1);
    
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}
/*************************************************************************
* windowStereo
*
* Windowing left or right channels
**************************************************************************/
void windowStereo(Word16 *input,      /* i: input L/R channel */
                  Word16 *mem_input,  /* i: mem input L/R */
                  Word16 *output      /* o: windoww input L/R channel */
                  )
{
   Word16 i;
   Word16 *ptr0, *ptr1;
   const Word16 *ptrwin;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (0) * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif

    ptr1 = output;
    /* zeroing first and last 11 points [0, 10] U [149,159]*/
    zero16(11, output);
    zero16(11, &output[NFFT -11]);

    /* windowing next 58 points [11, 68] <- mem [0, 57] *win [0, 57]*/
    ptr0 = mem_input;
    ptr1 += 11;
    ptrwin = win_D;
    FOR(i=11 ; i< 69; i++)
    {
        *ptr1++ = mult(*ptr0++, *ptrwin++);move16();
    }

    /* copy 22 middle points [69,90] <- input [0, 21] */
    ptr0 = input;
    mov16(22, ptr0, ptr1);

    ptr0 += 22;
    ptr1 += 22;
    /* windowing next 58 points [91, 148] <- input [22, 79] * win[57,0] */
    ptrwin = win_D+57;
    FOR(i=91 ; i< 149; i++)
    {
        *ptr1++ = mult(*ptr0++, *ptrwin--);move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* write_index
*
* write index in the stereo bitstream
**************************************************************************/
void write_index1(Word16* bpt_stereo, /* O */
                  Word16  index       /* I */
                  )
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = index & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}
void write_index2(Word16* bpt_stereo, /* O */
                  Word16  index       /* I */
                  )
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = shr(index,1);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = index & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void write_index3(Word16* bpt_stereo, /* O */
                  Word16  index       /* I */
                  )
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = shr(index,2);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,1);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = index & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void write_index4(Word16* bpt_stereo, /* O */
                  Word16  index       /* I */
                  )
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = shr(index,3);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,2);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,1);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = index & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void write_index5(Word16* bpt_stereo, /* O */
                  Word16  index       /* I */
                  )
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = shr(index,4);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,3);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,2);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = shr(index,1);
    tmp = tmp & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }

    tmp = index & 0x01; logic16();
    if(tmp == 0)
    {
        *bpt_stereo++ = G192_BITZERO; move16();
    }
    if(tmp != 0)
    {
        *bpt_stereo++ = G192_BITONE; move16();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void read_index1(Word16* bpt_stereo, /* I */
                 Word16* index       /* O */
                 )
{
    Word16 tmp1, tmp2, bit;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp1 = 0; move16();

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,1);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    *index = tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void read_index2(Word16* bpt_stereo, /* I */
                 Word16* index       /* O */
                 )
{
    Word16 tmp1, tmp2, bit;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp1 = 0; move16();

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,2);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,1);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    *index = tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void read_index3(Word16* bpt_stereo, /* I */
                 Word16* index       /* O */
                 )
{
    Word16 tmp1, tmp2, bit;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp1 = 0; move16();

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,4);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,2);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,1);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    *index = tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void read_index4(Word16* bpt_stereo, /* I */
                 Word16* index       /* O */
                 )
{
    Word16 tmp1, tmp2, bit;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp1 = 0; move16();

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,8);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,4);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,2);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,1);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    *index = tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

void read_index5(Word16* bpt_stereo, /* I */
                 Word16* index       /* O */
                 )
{
    Word16 tmp1, tmp2, bit;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp1 = 0; move16();

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,16);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,8);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,4);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,2);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    bit  = *bpt_stereo++;
    tmp2 = add(tmp1,1);
    if(sub(bit, G192_BITONE) == 0)
    {
        tmp1 = tmp2;
    }

    *index = tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* zero32
*
* write n (Word32)0 in array sx
**************************************************************************/
void zero32(Word16  n,    /* I : */
            Word32* sx    /* O : */
            )
{
    Word16 k;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR (k = 0; k < n; k++)
    {
        sx[k] = 0; move32();
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
}

/*************************************************************************
* spx_atan01
*
* fixed point atan()
*       Input:  x,   0< x < 1 (Q15)
*       Output: angle of atan(x) (Q13)
**************************************************************************/
static Word16 spx_atan01(Word16 x)
{
    Word16 x1,x2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    x1 = mult_r(M4,x);
    x2 = add(M3, x1) ;

    x1 = mult_r(x2, x);
    x2 = add(M2, x1);

    x1 = mult_r(x2, x);
    x2 = add(M1, x1);

    x1 = mult_r(x2, x);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return x1;
}

static Word16 calc_Phase0(Word32 L_num, Word32 L_den )
{
    Word16 iPhaseFix ; 
    Word16 norm_den;
    Word16 iTmpHi, iTmpLo;
    Word32 L_phase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    norm_den  = norm_l( L_den );
    L_den     = L_shl( L_den , norm_den );
    L_num     = L_shl( L_num , norm_den);
    iTmpLo    = L_Extract_lc( L_den, &iTmpHi);
    L_phase   = Div_32( L_num, iTmpHi ,iTmpLo );
    iTmpHi    = extract_h( L_phase);
    iPhaseFix = shr(spx_atan01(iTmpHi),3);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(iPhaseFix);
}

/*************************************************************************
* arctan2_fix32
*
* cases to compute the angle of phasei= atan(y/x) from the angle phase0= atan(min(|x|,|y|)/ max(|x|,|y|))
* first this angle phase0 is computed by the routine calc_Phase0 
* then to compute phasei from phase0 there are 8 cases: 
* depending of the signs of x and y, and whether |x]> |y|
* case0 : x>=0, |x]>= |y|, y>=0  phase0 = phase0 
* case1 : x>=0, |x]>= |y|, y< 0  phase1 = -phase0 
* case2 : x>=0, |x]<  |y|, y>=0  phase2 = PI/2 - phase0
* case3 : x>=0, |x]<  |y|, y< 0  phase3 = (phase0 - PI/2) = -phase2
* case4 : x<0,  |x]>= |y|, y>=0  phase4 = PI-phase0
* case5 : x<0,  |x]>= |y|, y< 0  phase5 = phase0-PI = -phase4
* case6 : x<0,  |x]<  |y|, y>=0  phase6 = PI/2 + phase0
* case7 : x<0,  |x]<  |y|, y< 0  phase7 = -phase0 - PI/2 = -phase6
*
* The even cases are addressed with a pointer on the array phaseCases
* ptr_phaseCases + 4 (pointer on one of the last  4 elements of phaseCases) indicates whether phase0 should be negated
* ptr_phaseCases (pointer on one of the first 4 elements of phaseCases ) addresses the offset angle to be added
* finally, for the odd cases the resulting phase is negated
* output (Q12)
**************************************************************************/
Word16 arctan2_fix32( Word32 y, Word32 x )
{
  Word32 L_num, L_den, L_temp;
  Word16 iPhase0;
  const Word16 *ptr_phaseCases;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  3 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
  ptr_phaseCases = phaseCases + 1;
  L_num= L_abs(x);
  L_den= L_abs(y);
  L_temp = L_sub(L_num, L_den); 
  IF(L_temp >= 0) 
  {
    ptr_phaseCases--;
    L_num= L_abs(y);
    L_den= L_abs(x);
  }
  if(x <0) 
  {
    ptr_phaseCases += 2;
  }

  if(L_num == 0)
  {
    iPhase0 = 0; move16();
  }

  IF(L_num != 0) 
  {
    iPhase0 = calc_Phase0(L_num, L_den);
  }
  if(*(ptr_phaseCases +4) != 0) iPhase0 = negate(iPhase0);
  iPhase0 = add(iPhase0, *(ptr_phaseCases));
  if(y<0) iPhase0 = negate(iPhase0);
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(iPhase0);
}

/*************************************************************************
* Round_Phase
*
* make sure that phase x belongs to interval [-pi,pi]
*       Input:  x
*       Output: x is limited to [-pi,pi] with modulo 2 pi
**************************************************************************/
Word16 Round_Phase(Word16 x)
{
  Word16 tmp,tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
  tmp  = sub(x, NGPI_FQ12);
  tmp1 = sub(x, PI_FQ12);
    
  if( tmp < 0)
  {
    x = add(x, PI2_FQ12);
  }
  if(tmp1 > 0)
  {
    x = sub(x, PI2_FQ12);
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(x);
}

/*************************************************************************
* spx_cos
*
* fixed point cos()
*       Input:  x (Q12)
*       Output: cos(x) (Q15)
**************************************************************************/
Word16  spx_cos(Word16 x)
{
  Word16 x2;
  Word16 Tmp_V;
  Word16 tmp,tmp1;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
  x = Round_Phase(x);
  x = shl( abs_s(x), 1) ; // Q13
  tmp = sub(x,12868);

  if(tmp >= 0)
  {
    x = sub( 25736 , x );
  }
  x2    = extract_l( L_shr( L_mult(x,x), 14) );       // Q 13 

  Tmp_V = extract_l( L_shr( L_mult(K4,x2), 14) );     // Q 13 
  Tmp_V = add( Tmp_V , K3 );
  Tmp_V = extract_l( L_shr( L_mult(Tmp_V,x2), 14) );  // Q 13 
  Tmp_V = add( Tmp_V , K2 );
  Tmp_V = extract_l( L_shr( L_mult(Tmp_V,x2), 14) );  // Q 13 

  tmp1  = add( Tmp_V , K1 );
  if(tmp >= 0)
  {
    tmp1 = negate(tmp1);
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return shl(tmp1,2);//Q15
}
#endif /*LAYER_STEREO*/
