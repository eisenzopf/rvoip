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
/* common routines to swb_encode_avq and swb_decode_avq  */
/* (formely local routines in swb_encode_avq.c Software Release 1.00 (2010-09))*/

#include "bit_op.h"
#include "bwe.h"
#include "avq.h"
#include "math_op.h"
#include "rom.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

Word16 getBandLx_decodAVQ(Word16 *smdct_coef_Lx, Word16 *smdct_coef_AVQ, Word16 *bandTmp, Word16 nbBand, Word16 *bandLx, Word16 *bandZero)
{
  Word16 ib, cntLx;
  Word16 *ptr_Lx, *ptr, *ptra, *ptrb, *ptrc;
  Word32 L_en;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + SIZE_Word32 + 5 * SIZE_Ptr), "dummy");
#endif

  ptr_Lx = smdct_coef_Lx;
  ptra = bandTmp;  
  ptrb = bandLx;
  ptrc = bandZero;
  cntLx = 0; move16();
  FOR( ib=0; ib<nbBand; ib++ )
  {
    L_en = Sum_vect_E8(ptr_Lx); 
    IF( L_en == 0 )
    {
      *ptrc++ = *ptra++; move16();
    }
    ELSE
    {                                 
      cntLx = add(cntLx, 1);
      ptr = smdct_coef_AVQ + shl(*ptra , 3);
      array_oper(WIDTH_BAND, QCOEF, ptr_Lx, ptr, &shl);
      *ptrb++ = *ptra++; move16();
    }
    ptr_Lx += WIDTH_BAND;
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return (cntLx);
}

void sortIncrease(
                         Word16 n,       /* i  : array dimension */
                         Word16 nbMin,   /* i  : number of minima to sort */
                         Word16 *xin,    /* i  : arrray to be sorted */ 
                         Word16 *xout    /* o  : sorted array  */
                         )
{
  Word16 i, j, xtmp[N_SV], xmin, pos;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((4 + N_SV) * SIZE_Word16), "dummy");
#endif
  /*****************************/
  FOR (i=0; i<n;i++)
  {
    xtmp[i] = xin[i];                             move16();
  }
  FOR (i=0; i<nbMin; i++)
  {
    xmin  = xtmp[0];                              move16();
    pos = 0;                                      move16();
    FOR (j=1; j<n; j++)
    {
      if (sub(xtmp[j], xmin) < 0)
      {
        pos = j;                                  move16();
      }
      xmin = s_min(xtmp[j], xmin);
    }
    xout[i] = xtmp[pos];                          move16();
    xtmp[pos] = MAX_16;                           move16();
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
  return;
}

/* ***** try to find a filling of zero subbands ***** */
void getBaseSpectrum_flg0 (Word16 *avqType, Word16 *smdct_coef_avq, Word16 *svec_base)
{
  Word16 bandLoc[N_SV_L1+N_SV_L2];
  Word16 ib, i;
  Word16 *ptr0, *ptr1;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((2 + N_SV_L1 + N_SV_L2) * SIZE_Word16 + 2 * SIZE_Ptr), "dummy");
#endif
  i = avqType[0];move16();
  mov16(i, &avqType[3], bandLoc);
  mov16(avqType[1], &avqType[6], &bandLoc[i]);
  i = add(i,avqType[1]);
  sortIncrease(i, 3, bandLoc, bandLoc);
  ptr1 = svec_base;
  FOR(ib=0; ib<3; ib++) 
  {
    i = bandLoc[ib]; move16();
    ptr0 = smdct_coef_avq + shl(i,3);
    mov16(WIDTH_BAND, ptr0, ptr1);
    ptr1 += WIDTH_BAND;
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

Word16 computeGainFillBand(Word16 *ptr_svec_base, Word16 expx)
{
    Word16 Gain16 ; 
    Word32 L_en, L_tmp;
    Word16 exp_den; 
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 + 2*SIZE_Word32 ), "dummy");
#endif

    L_en = Sum_vect_E8( ptr_svec_base);
    L_tmp = norm_l_L_shl(&exp_den, L_en);
    exp_den = sub(16, exp_den); /* 16 for round */
    L_tmp = Isqrt_lc(L_tmp, &exp_den);
    Gain16 = extract_h_L_shr_sub(L_tmp, sub(expx,3),exp_den);/*Q15*/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return(Gain16);
}

/* backward reordering */
void bwdReorder (const Word16 *senv_BWE, Word16 *scoef_SWB_AVQ, Word16 *smdct_coef, const Word16 *ord_bands, Word16 *avqType)
{
  Word16 ib, i, j, l;
  Word16 *ptr0, *ptr1, *ptr2;
  Word16 bandLoc[N_SV];
  Word32 L_tmp;
  Word16 inc, iavq, nbBand, *ptrBand;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((7 + N_SV) * SIZE_Word16 + SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif

  inc = 3; move16();
  ptrBand = avqType + inc;
  FOR(iavq=0; iavq<3; iavq++) 
  {
    nbBand = avqType[iavq];  move16();
    ptr2 = bandLoc;
    FOR( ib=0; ib<nbBand; ib++ )
    {
      i = ptrBand[ib]; move16();
      j = ord_bands[i]; move16();
      *ptr2++ = j; move16();
      ptr0 = scoef_SWB_AVQ+ shl(i, 3);
      ptr1 = smdct_coef+ shl(j, 3);
      FOR( l=0; l<WIDTH_BAND; l++)
      {
        L_tmp = L_mult(ptr0[l], senv_BWE[j]);
        ptr1[l]= round_fx_L_shl(L_tmp, 15-QCOEF);
        move16();
      }
    }
    mov16(nbBand, bandLoc, ptrBand);
    ptrBand += inc;
    inc = add(inc, 3);
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}

Word16 getNbBitdetprob_flg (Word16 unbits_L1)
{
    Word16 nb;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (SIZE_Word16), "dummy");
#endif
    nb = 0; move16();
    IF(unbits_L1 > 0) 
    {
        nb = add(nb, 1);
        IF( sub(unbits_L1,1) > 0 )
        {
            nb = add(nb, 1);
            if( sub(unbits_L1 ,N_BITS_FILL_L1+1 ) == 0) 
            {
                nb = sub(nb, 1);
            }
        }
    }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return(nb);
}

Word16 getNbZero(Word16 *ptr0)
{
    Word16 j;
    Word16 nbZero;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2*SIZE_Word16), "dummy");
#endif

    nbZero = 0; move16();
    FOR(j=0; j<WIDTH_BAND; j++)
    {
        if(*ptr0 == 0) {
            nbZero = add(nbZero, 1);
        }
        ptr0++;
    }

    move16();
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
    return(nbZero);
}
