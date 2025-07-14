/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

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

#define QCOEF 9

/*------------------------------------------------------------------------*
* Prototypes
*------------------------------------------------------------------------*/
Word32  Sum_vect_E(const Word16 *vec, const Word16 lvec);   /* DEFINED IN fec_low_band.c */

static void decoder_coef_SWB_AVQ_adj( 
                                     const Word16   zero_vector[],  /* o:  output vector signalising zero bands     */
                                     const Word16   sord_b[],       /* i:  percept. importance  order of subbands   */
                                     Word16   coef_SWB[],     /* i/o:  locally decoded MDCT coefs.             */ /* Q(scoef_SWBQ) */
                                     UWord16  *pBst,           /* i/o:  pointer to bitstream buffer            */
                                     Word16  *unbits,
                                     const Word16   smode,          /* 1: L1 / 2: L2                                */
                                     const Word16   N_BITS_AVQ
                                     );

void if_negate(Word16 *scoef_SWB, Word16 en)
{
  if(*scoef_SWB < 0)
  {
    en =negate(en);
  }    
  *scoef_SWB = en; move16();

  return;
}

/* Constructor for AVQ decoder */
void* avq_decode_const (void)
{
  AVQ_state_dec *dec_st = NULL;

  dec_st = (AVQ_state_dec *) malloc (sizeof(AVQ_state_dec));
  if (dec_st == NULL) return NULL;

#ifdef DYN_RAM_CNT
#ifdef MEM_STT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((SWB_F_WIDTH*5 + N_SV + 3)*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
#endif

  avq_decode_reset ((void *)dec_st);

  return (void *)dec_st;
}

void avq_decode_dest (void *work)
{
  AVQ_state_dec *dec_st = (AVQ_state_dec *)work;

  if (dec_st != NULL)
  {
    free (dec_st);
  }

#ifdef DYN_RAM_CNT
#ifdef MEM_STT
  DYN_RAM_POP();
#endif
#endif

}

Word16 avq_decode_reset (void *work)
{
  AVQ_state_dec *dec_st = (AVQ_state_dec *) work;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif  

  if (dec_st != NULL)
  {
    zero16(sizeof(AVQ_state_dec)/2, work);

    dec_st->pre_cod_Mode = NORMAL; move16();
    dec_st->pre_scoef_SWBQ0 = 15; move16();
    dec_st->pre_scoef_SWBQ1 = 15; move16();
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif

  return DECODER_OK;
}

void bwe_avq_buf_reset(void *work)
{   
  AVQ_state_dec *dec_st = (AVQ_state_dec *) work;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif
  zero16(N_SV, dec_st->prev_zero_vector);
  zero16(SWB_F_WIDTH, dec_st->sprefSp); 
  zero16(SWB_F_WIDTH, dec_st->sbuffAVQ); 
  zero16(SWB_F_WIDTH, dec_st->spreAVQ0); 
  zero16(SWB_F_WIDTH, dec_st->spreAVQ1); 

  dec_st->pre_scoef_SWBQ0 = 15; move16();
  dec_st->pre_scoef_SWBQ1 = 15; move16();
  dec_st->pre_cod_Mode = NORMAL; move16();
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
}

void decoder_SWBL1L2_AVQ( 
                         void*                 p_AVQ_state_dec,      /* (i/o): Work space       */
                         UWord16 *pBst_L1,    /* i:	Input bitstream for SWBL1				*/
                         UWord16 *pBst_L2,    /* i:	Input bitstream for SWBL2				*/
                         const Word16 layers,      /* i:	number of SWB layers received			*/
                         const Word16 senv_BWE[],  /* i:	Input normalized frequency envelope		*/
                         const Word16 *ord_bands,  /* i:	percept. importance	order of subbands   */
                         Word16 zero_vector[], /* o:	Output vector signalising zero bands	*/
                         Word16 scoef_SWB_AVQ[],/* o:	Output MDCT coefficients from AVQ		*/
                         const Word16 flg_bit,
                         Word16 *unbits_L1,
                         Word16 *unbits_L2
                         )
{
  Word16 i, j, tmp16;

  Word16 zero_vector_tmp[N_SV];    /* = 0 when zero subband, = 1 when L1 coeff. applied, = 2 when L2 coeff. applied */
  Word16 Nsv_L2;
  UWord16 *bptpt;
  Word32 L_en, L_tmp;
  Word16 smdct_coef_L1[WIDTH_BAND*N_SV_L1], smdct_coef_L2[WIDTH_BAND*N_SV_L2], scoef_dec[SWB_F_WIDTH];

  Word16 svec_base[N_SV*WIDTH_BAND];
  Word16 cnt_used, cnt_unused, ind_corr_max=0;
  Word16 Gain16, exp_den;

  Word16 i8, k8;

  AVQ_state_dec *w_AVQ_state_dec = (AVQ_state_dec *)p_AVQ_state_dec;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Ptr + ( 11 + N_SV + WIDTH_BAND*(N_SV+N_SV_L1+N_SV_L2) + SWB_F_WIDTH ) * SIZE_Word16 + 2 * SIZE_Word32), "dummy");
#endif
  /*****************************/
  zero16( N_SV, zero_vector_tmp );

  /* read and decode AVQ parameters from SWBL1 */
  *unbits_L1 = AVQ_Demuxdec_Bstr( pBst_L1, smdct_coef_L1, N_BITS_AVQ_L1, N_SV_L1 ); move16();

  IF(sub(flg_bit, 2)==0)
  {
    *unbits_L1 = add(*unbits_L1, 1);
  }
  
  /* find zero subbands */
  Nsv_L2 = 0; move16();
  FOR( i = 0; i < N_SV_L1; i++ )
  {
    L_en = Sum_vect_E( smdct_coef_L1 + shl(i, 3), WIDTH_BAND );
    if( L_en == 0 )
    {
      Nsv_L2 = add(Nsv_L2, 1);
    }
    if(L_en != 0)
    {
      zero_vector_tmp[i] = 1; move16();
    }
  }
  tmp16 = sub(layers, 2);
  IF( tmp16 == 0 )
  {
    *unbits_L2 = AVQ_Demuxdec_Bstr( pBst_L2, smdct_coef_L2, N_BITS_AVQ_L2, N_SV_L2 ); move16();
  }
  /* reconstruct SWBL1 (and SWBL2) MDCT coefficients */
  k8 = 0; move16();
  zero16( SWB_F_WIDTH, scoef_dec );
  FOR( i = 0; i<N_SV; i++ )
  {
    i8 = shl(i, 3);

    test();
    IF( (zero_vector_tmp[i] == 0) && tmp16 == 0 )
    {
      IF( sub(k8, N_SV_L2*8) < 0 )
      {
        L_en = Sum_vect_E( smdct_coef_L2 + k8, WIDTH_BAND );
        IF (L_en > 0)
        {
          zero_vector_tmp[i] = 2; move16();
          array_oper(WIDTH_BAND, QCOEF, &smdct_coef_L2[k8], &scoef_dec[i8], &shl);
        }
      }
      k8 = add(k8, 8);
    }
    ELSE IF( sub(zero_vector_tmp[i], 1) == 0 )
    {
      array_oper(WIDTH_BAND, QCOEF, &smdct_coef_L1[i8], &scoef_dec[i8], &shl);
    }
  }
  i8 = sub(flg_bit, 1);
  /* read detzer_flag */
  IF( tmp16 == 0 )
  {
    test();
    IF( i8 != 0 && (*unbits_L1 > 0) )
    {
      IF( sub(flg_bit, 2) != 0 )
      {
        bptpt = pBst_L1 + sub(N_BITS_AVQ_L1, *unbits_L1);
      }
      ELSE
      {
        bptpt = pBst_L1 + sub(N_BITS_AVQ_L1_PLS, *unbits_L1);
      }

      w_AVQ_state_dec->detzer_flg = GetBit( &bptpt, 1 );
      *unbits_L1 = sub(*unbits_L1, 1);
    }
    test();

    IF( (w_AVQ_state_dec->detzer_flg > 0) && i8 == 0 )
    {
      w_AVQ_state_dec->detzer_flg = add(w_AVQ_state_dec->detzer_flg, 1);

      if( sub(w_AVQ_state_dec->detzer_flg, 5) >= 0)
      {
        w_AVQ_state_dec->detzer_flg = 0; move16();
      }
    }
    test();

    if( i8 != 0 && (w_AVQ_state_dec->detzer_flg > 0) )
    {
      w_AVQ_state_dec->detzer_flg = 1; move16();
    }

  }
  ELSE
  {
    w_AVQ_state_dec->detzer_flg = 0; move16();
  }

  test();test();
  IF( i8 != 0 && tmp16 == 0 && (w_AVQ_state_dec->detzer_flg==0) )
  {
    ind_corr_max = 0; move16();
    /* prepare vectors */
    cnt_unused = 0;   move16();
    cnt_used = 0;     move16();
    FOR( i=0; i<N_SV; i++ )
    {
      if( zero_vector_tmp[i] == 0 )
      {
        cnt_unused = add(1, cnt_unused);
      }
      if( zero_vector_tmp[i] != 0 )
      {
        i8 = shl(i, 3);
        k8 = shl(cnt_used, 3);
        mov16(WIDTH_BAND, &scoef_dec[i8],&svec_base[k8]); 
        cnt_used = add(1, cnt_used);
      }
    }

    tmp16 = 0;			move16();	/* tmp flag for L2 filling */
    /* reconstruct the zero subband 1 */
    test();
    IF( (sub(*unbits_L1, N_BITS_FILL_L1)>=0) && (sub(cnt_used, N_BASE_BANDS) >= 0) )
    {
      /* read from the bitstream */
      IF( sub(flg_bit, 2) != 0 )
      {
        bptpt = pBst_L1 + sub(N_BITS_AVQ_L1, *unbits_L1);
      }
      ELSE
      {
        bptpt = pBst_L1 + sub(N_BITS_AVQ_L1_PLS, *unbits_L1);
      }

      ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L1 );
      *unbits_L1 = sub(*unbits_L1, N_BITS_FILL_L1); move16();

      IF( sub(ind_corr_max, CORR_RANGE_L1) < 0 )
      {
        FOR( i=0; i<N_SV; i++ )
        {
          IF( zero_vector_tmp[i] == 0 )
          {
            L_en = Sum_vect_E( &svec_base[ind_corr_max], WIDTH_BAND );
            L_tmp = norm_l_L_shl(&exp_den, L_en);
            exp_den = sub(16, exp_den); /* 16 for round */
            L_tmp = Isqrt_lc(L_tmp, &exp_den);
            Gain16= extract_h_L_shr_sub(L_tmp, -3,exp_den);   /*Q15 */

            i8 = shl(i, 3);
            IF( sub(Gain16,32767) < 0 )
            {
              array_oper(WIDTH_BAND, Gain16, &svec_base[ind_corr_max], &scoef_dec[i8], &mult_r);
            }
            ELSE
            {
              mov16(WIDTH_BAND, &svec_base[ind_corr_max], &scoef_dec[i8]);
            }
            zero_vector_tmp[i] = 2;       move16();
            BREAK;
          }
        }
      }
      tmp16 = ind_corr_max;			move16();	/* tmp flag for L2 filling */
    }

    /* reconstruct the zero subband 2 */
    test();test();test();
    IF( sub(cnt_used, N_BASE_BANDS) >= 0 && sub(*unbits_L2, N_BITS_FILL_L2)>=0 && sub(cnt_unused, 1)>0 && (w_AVQ_state_dec->detzer_flg==0) )
    {
      /* read from the bitstream */
      bptpt = pBst_L2 + sub(N_BITS_AVQ_L2, *unbits_L2);

      ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L2 );
      *unbits_L2 = sub(*unbits_L2, N_BITS_FILL_L2); move16();

      IF( sub(ind_corr_max, CORR_RANGE_L2 ) < 0)
      {
        FOR( i=0; i<N_SV; i++ )
        {
          IF( zero_vector_tmp[i] == 0 )
          {
            IF( sub(tmp16, CORR_RANGE_L1) < 0 )
            {
              L_en = Sum_vect_E( &svec_base[ind_corr_max], WIDTH_BAND );
              L_tmp = norm_l_L_shl(&exp_den, L_en);
              exp_den = sub(16, exp_den); /* 16 for round */
              L_tmp = Isqrt_lc(L_tmp, &exp_den);
              Gain16= extract_h_L_shr_sub(L_tmp, -3,exp_den);   /*Q15 */

              i8 = shl(i, 3);
              IF( sub(Gain16,32767) < 0 )
              {
                array_oper(WIDTH_BAND, Gain16, &svec_base[ind_corr_max], &scoef_dec[i8], &mult_r);
              }
              ELSE
              {
                mov16(WIDTH_BAND, &svec_base[ind_corr_max], &scoef_dec[i8]);
              }
              zero_vector_tmp[i] = 2;     move16();
              BREAK;
            }
            tmp16 = 0;
          }
        }
      }
    }
  }
  /* backward reordering of subbands */
  FOR( i=0; i<N_SV; i++ )
  {
    mov16( WIDTH_BAND, &scoef_dec[shl(i, 3)], &scoef_SWB_AVQ[shl(ord_bands[i], 3)] );
    zero_vector[ord_bands[i]] = zero_vector_tmp[i];    move16();
  }
  /* denormalization per band */
  FOR( i=0; i<N_SV; i++ )
  {
    i8 = shl(i, 3);
    FOR( j=0; j<WIDTH_BAND; j++ )
    {
      /*coef_SWB_AVQ[j] *= Fenv_BWE[i];*/
      L_tmp = L_mult(scoef_SWB_AVQ[i8+j], senv_BWE[i]);
      scoef_SWB_AVQ[i8+j]= round_fx_L_shl(L_tmp, 15-QCOEF);
      move16();
    }

  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/  
  return;
}

/*--------------------------------------------------------------------------*
*  Function  swbl1_decode_AVQ()		                                         *
*  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~                                            *
*  Main function for decoding Extension layers SWBL1 and SWBL2             *
*--------------------------------------------------------------------------*/
void swbl1_decode_AVQ (
                       void*                 p_AVQ_state_dec,      /* (i/o): Work space       */
                       UWord16  *pBst_L1,       /* i:	Input bitstream for SWBL1                           */
                       UWord16  *pBst_L2,       /* i:	Input bitstream for SWBL2                           */
                       const Word16  *sfEnv_BWE,     /* i:	Input frequency envelope from SWBL0	Q(scoef_SWBQ)   */
                       Word16  *scoef_SWB,     /* i/o:	Output SWB MDCT coefficients      Q(scoef_SWBQ)   */
                       const Word16   index_g_5bit,  /* i:	5 bit index of frame gain from SWBL0                */
                       const Word16   cod_Mode,      /* i:	mode information from SWBL0                         */
                       const Word16   layers,        /* i:	number of swb layers received                       */
                       Word16  *scoef_SWBQ
                       ) 
{
  Word16  i, j, index_gain;
  Word16  pos1;
  Word16  unbits_L1, unbits_L2;
  UWord16 *bptpt;

  Word16  zero_vector[N_SV];
  Word16 ip[N_SV];
  Word16 senv_BWE[N_SV];             /* Q(12) */
  Word16 scoef_SWB_AVQ[SWB_F_WIDTH]; /* Q(12) */
  Word16 sord_b[N_SV], tmp16, tmp16_2;
  Word16 flg_bit;                

  Word16 sGainBWE   =  16384;  /* Q(sGainBWEQ):14-index_g_5bit */ /* sGainBWE can be calculated using shift operation because significand is 1 */
  Word16 sGopt;                /* Q(sGainBWEQ):14-index_g_5bit */
  Word16 sGainBWEQ  =  sub( 14, index_g_5bit);
  Word32 L_temp     =  0;
  Word16 k, en;

  Word16 sFenv_BWE[N_SV], sEnv_Q;
  Word16 senv_avrg[N_SV], norm_ltmp, smin_coef;

  Word16 diff_Q, i8, stmp, sbit;
  Word32 lbuff_coef_pow, lbuff_Fenv_BWE, ltmp;
  UWord16 *bptpt_L1, *bptpt_L2;

  Word16 temp1, temp2, temp3;
  Word16 *ptr0, *ptr1;
  Word16 scoef_SWBQ1, scoef_SWBQ2, temp, pos2;

  AVQ_state_dec *w_AVQ_state_dec = (AVQ_state_dec *)p_AVQ_state_dec;

  zero16(N_SV, zero_vector);
  zero16(N_SV, ip);
  zero16(N_SV, senv_BWE);
  zero16(SWB_F_WIDTH, scoef_SWB_AVQ);
  zero16(N_SV, sord_b); 
  zero16(N_SV, sFenv_BWE);
  zero16(N_SV, senv_avrg);
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32	ssize;
    ssize = (UWord32) ((28 + 6 * N_SV + SWB_F_WIDTH) * SIZE_Word16);
    ssize += (UWord32) (6 * SIZE_Ptr);
    ssize += (UWord32) (4 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  /* calculate subband energy */
  sEnv_Q = add (*scoef_SWBQ, index_g_5bit); /* Q(sEnv_Q=scoef_SWBQ+index_g_5bit):identical to sYfb_Q in encoder */
  loadSubbandEnergy ((Word16)cod_Mode, (Word16 *)sfEnv_BWE, sFenv_BWE);

  /* order subbands by decreasing perceptual importance  */
  mov16(N_SV, sFenv_BWE, ip);

  Sort( ip, N_SV, sord_b, senv_avrg );   /* senv_avrg used as tmp buffer only */

  diff_Q = sub(sEnv_Q, 12);
  array_oper(N_SV, diff_Q, sFenv_BWE, senv_BWE, &shr);

  flg_bit = 0; move16();
  /* (Word16)cod_Mode == NORMAL && w_AVQ_state_dec->pre_cod_Mode == NORMAL */
  IF (add((Word16)cod_Mode, w_AVQ_state_dec->pre_cod_Mode) == NORMAL)
  {   
    /*----------------------------------------------------------------------
    *
    * AVQ with mode switching
    *
    *---------------------------------------------------------------------*/

    /* get flg_bit from bitstream */
    /* -------------------------- */
    IF (sub(*pBst_L1++, ITU_G192_BIT_0) == 0)
    {
      /* ---------------------------------------------------------------- */
      /* - decoding 0                                                     */
      /* ---------------------------------------------------------------- */

      /* ***** decode SWBL1 and SWBL2 AVQ parameters ***** */
      decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, senv_BWE, sord_b, zero_vector, scoef_SWB_AVQ 
        , 0, &unbits_L1, &unbits_L2);
    }
    ELSE
    {
      /* ----------------------------------------------------------------- */
      /* - decoding 1                                                      */
      /* ----------------------------------------------------------------- */
      Word16 cnt;
      Word16 stmp_coef_SWB_AVQ[SWB_F_WIDTH]; /* Q12 */
      Word16 scoef_SWB_AVQ_abs[SWB_F_WIDTH]; /* Q12 */   
      Word16 senv_BWE_mod[N_SV];         /* Q(12) */

      zero16( SWB_F_WIDTH, stmp_coef_SWB_AVQ);
      zero16( SWB_F_WIDTH, scoef_SWB_AVQ_abs);
      zero16( N_SV, senv_BWE_mod);

      /*****************************/
#ifdef DYN_RAM_CNT
      DYN_RAM_PUSH((UWord32) ((1 + N_SV + 2 * SWB_F_WIDTH) * SIZE_Word16), "dummy");
#endif
      /*****************************/

      flg_bit = 1;
      move16();

      /* calculate Fenv_BWE_mod */
      /* ---------------------- */
      array_oper(N_SV, 19661, senv_BWE, senv_BWE_mod, &mult);/* Q(12): Q(15) + Q(12) + 1 - 16 */

      /* ***** decode SWBL1 and SWBL2 AVQ parameters ***** */
      decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, senv_BWE_mod, sord_b, zero_vector, scoef_SWB_AVQ 
        , 1, &unbits_L1, &unbits_L2);

      mov16(SWB_F_WIDTH, scoef_SWB_AVQ, stmp_coef_SWB_AVQ);

      /* compute coef_SWB_AVQ */
      /* -------------------- */
      FOR (i=0; i<N_SV; i++)
      {
        smin_coef = MAX_16; /* Q12 */		move16();	

        IF (zero_vector[i] != 0) 
        {
          i8 = shl (i, 3);

          /* calculate abs. value of coef_SWB_AVQ */
          cnt = 0; move16 ();
          lbuff_coef_pow = 0L; move32 ();

          senv_avrg[i] = MAX_16; move16 (); /* Q12 */
          FOR (j=0; j<WIDTH_BAND; j++)
          {
            IF (scoef_SWB_AVQ[i8+j] != 0)
            {
              stmp = scoef_SWB_AVQ[i8+j];		move16();	
              if (stmp < 0) stmp = negate (stmp);

              if (sub (stmp, senv_avrg[i]) < 0)
              {
                senv_avrg[i] = stmp; move16 (); /* Q12 */
              }
              scoef_SWB_AVQ_abs[i8+j] = add (stmp, shr (senv_BWE[i], 1)); move16 (); /* Q12 */

              stmp = shr (scoef_SWB_AVQ_abs[i8+j], 3); /* Q9:12-3 */
              lbuff_coef_pow = L_mac (lbuff_coef_pow, stmp, stmp); /* Q19:9+9+1 */
              cnt = add (cnt, 1);

              /* multiply sign inf. */
              stmp = scoef_SWB_AVQ_abs[i8+j]; move16 (); /* Q12 */
              if (scoef_SWB_AVQ[i8+j] < 0)
              {
                stmp = negate (stmp);
              }
              scoef_SWB_AVQ[i8+j] = stmp; move16 ();

              if (sub (scoef_SWB_AVQ_abs[i8+j], smin_coef) < 0)
              {
                smin_coef = scoef_SWB_AVQ_abs[i8+j]; move16 (); /* Q12 */
              }
            }
          }

          /* calculate buff_Fenv_BWE */
          ltmp = L_mult (senv_BWE[i], senv_BWE[i]); /* Q22:12+12+1-3 */

          ltmp = L_shr (ltmp, 3); /* Q19:22-3 */
          ltmp = L_sub (ltmp, lbuff_coef_pow); /* Q19 */
          ltmp = norm_l_L_shl(&norm_ltmp, ltmp);
          lbuff_Fenv_BWE = 0; move32 ();
          IF (ltmp > 0)
          {
            lbuff_Fenv_BWE = L_shr (L_mult (round_fx (ltmp), dentbl[cnt]), norm_ltmp); /* Q19:19+norm_ltmp-16+15+1-norm_ltmp */

            SqrtI31 (lbuff_Fenv_BWE, &lbuff_Fenv_BWE); /* Q25:31-(31-19)/2 */

            ltmp = L_mult0 (senv_BWE[i], 4096); /* Q25:12+13 */
            lbuff_Fenv_BWE = L_min(lbuff_Fenv_BWE, ltmp); /* Q25 */
            IF (sub (zero_vector[i], 1) == 0)
            {
              ltmp = L_mult0 (smin_coef, 1024); /* Q25:12+13 */
              test (); test (); test ();
              IF ( L_sub (lbuff_Fenv_BWE, ltmp) > 0 && sub (cnt, 1) == 0)
              {
                lbuff_Fenv_BWE = ltmp; move32 (); /* Q25 */
              }
              ELSE IF ( L_sub (lbuff_Fenv_BWE, L_shl (ltmp, 1)) > 0 && sub (cnt, 2) == 0)
              {
                lbuff_Fenv_BWE = L_shl (ltmp, 1); /* Q25 */
              }
              ELSE IF ( L_sub (lbuff_Fenv_BWE, L_shl (ltmp, 2)) > 0 && sub (cnt, 4) == 0)
              {
                lbuff_Fenv_BWE = L_shl (ltmp, 2); /* Q25 */
              }
            }
          }

          /* calculate abs. value of coef_SWB_AVQ */
          stmp = round_fx (L_shl (lbuff_Fenv_BWE, 3)); /* Q12:25+3-16 */
          FOR (j=0; j<WIDTH_BAND; j++)
          {
            IF (scoef_SWB_AVQ[i8+j] == 0)
            {
              tmp16 = stmp; move16();
              if (scoef_SWB[i8+j] < 0)
              {
                tmp16 = negate (stmp);
              }
              scoef_SWB_AVQ[i8+j] = tmp16; move16 (); /* Q12 */
            }
          }
        }/* end of if(zero_vector[i] != 0) */
        ELSE
        {
          senv_avrg[i] = shl (mult_r (senv_BWE[i], 29491), 1); /* Q12:12+14+1-16+1 */
          move16();
        }
      }

      IF( sub(layers,2) == 0)
      {
        Word16 svec_base[N_SV*WIDTH_BAND];
        Word16 cnt_used, cnt_unused, ind_corr_max;
        Word16 iGain16, den, exp_den, exp_num, tmp16, tmp16_2, Gain16; 
        Word32 L_tmp, L_en;
        Word16 detprob_flg;

        /*****************************/
#ifdef DYN_RAM_CNT
        {
          UWord32	ssize;
          ssize = ((N_SV*WIDTH_BAND + 3+7+1)*SIZE_Word16);
          ssize += (2*SIZE_Word32);
          DYN_RAM_PUSH(ssize, "dummy");
        }
#endif
        /*****************************/

        /* read 'detprob_flg' from the L1 bitstream */
        detprob_flg = 0;      move16();
        bptpt = pBst_L1 + sub(N_BITS_AVQ_L1, unbits_L1);      move16();
        IF( sub(unbits_L1,N_BITS_FILL_L1+1) > 0)
        {
          detprob_flg = GetBit( &bptpt, 2 );
          unbits_L1 = sub(unbits_L1,2);
        }
        ELSE IF( unbits_L1 > N_BITS_FILL_L1 )
        {
          detprob_flg = GetBit( &bptpt, 1 );
          unbits_L1 = sub(unbits_L1,1);
        }
        ELSE IF( sub(unbits_L1,1) > 0 )
        {
          detprob_flg = GetBit( &bptpt, 2 );
          unbits_L1 = sub(unbits_L1,2);
        }
        ELSE IF( unbits_L1 > 0 )
        {
          detprob_flg = GetBit( &bptpt, 1 );
          unbits_L1 = sub(unbits_L1,1);
        }

        /* prepare vectors */
        cnt_unused = 0;         move16();
        cnt_used = 0;         move16();
        FOR( i=0; i<N_SV; i++ )
        {
          IF( zero_vector[i] == 0 )
          {
            cnt_unused = add(cnt_unused,1);
            IF( sub(detprob_flg,1) == 0 )
            {
              senv_BWE[i] = mult_r(senv_BWE[i], 16384);   move16();
            }
            IF( sub(detprob_flg,2) == 0 )
            {
              senv_BWE[i] = mult_r(senv_BWE[i], 8192);   move16();
            }
            IF( sub(detprob_flg,3) == 0 )
            {
              senv_BWE[i] = mult_r(senv_BWE[i], 4096);   move16();
            }
          }
          ELSE
          {
            exp_den = norm_s(senv_BWE[i]);
            den = shl(senv_BWE[i], exp_den);

            iGain16 = 16384;
            move16();
            if (sub(16384, den) <= 0)
            {
              iGain16 = div_s(16384, den);
            }
            exp_num = sub(14, add(exp_den,12));  /* normalized smdct_coef in 12 -> gain in Q15 */
            /*cnt_used*WIDTH_BAND*/
            tmp16 = shl(cnt_used,3);
            /*i*WIDTH_BAND*/
            tmp16_2 = shl(i,3);
            FOR( j=0; j<WIDTH_BAND; j++ )
            {
              svec_base[tmp16+j] = round_fx_L_shr_L_mult(scoef_SWB_AVQ[tmp16_2+j], iGain16, exp_num);
              move16();
            }
            cnt_used = add(cnt_used,1);
          }
        }

        k = 0;			move16();	/* tmp flag for L2 filling */
        /* reconstruct the zero subband 1 */
        test();
        IF( (sub(cnt_used,N_BASE_BANDS) >= 0) && (sub(unbits_L1,N_BITS_FILL_L1)>=0) )
        {
          /* read from the bitstream */
          bptpt = pBst_L1 + sub(N_BITS_AVQ_L1, unbits_L1);        move16();
          ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L1 ); 
          unbits_L1 = sub(unbits_L1, N_BITS_FILL_L1);

          IF( sub(ind_corr_max,CORR_RANGE_L1) < 0)
          {
            FOR( i=0; i<N_SV; i++ )
            {
              IF( zero_vector[i] == 0 )
              {
                /*correct_rat = 1/Sqrt(sum_vect_E( &vec_base[ind_corr_max], WIDTH_BAND )/WIDTH_BAND);*/
                L_en = Sum_vect_E( &svec_base[ind_corr_max], WIDTH_BAND );
                L_tmp = norm_l_L_shl(&exp_den, L_en);
                exp_den = sub(16, exp_den); /* 16 for round */
                L_tmp = Isqrt_lc(L_tmp, &exp_den);
                Gain16= extract_h_L_shr_sub(L_tmp, -6,exp_den);   /*Q15 */

                IF( sub(Gain16, 32767)< 0 )
                {
                  Gain16 = mult_r(Gain16,senv_BWE[i]);    move16(); /* Q15*Q12 ->Q12 */
                }
                ELSE
                {
                  Gain16 = senv_BWE[i];    move16();    /* Q12 -> Q12 */
                }
                i8 = shl(i, 3);
                FOR( j=0; j<WIDTH_BAND; j++ )
                {
                  scoef_SWB_AVQ[i8+j] = round_fx_L_shl_L_mult(svec_base[ind_corr_max+j],Gain16,3);  
                  move16();
                }
                zero_vector[i] = 2;                 move16();
                BREAK;
              }
            }
          }
          k = ind_corr_max;			move16();	/* tmp flag for L2 filling */
        }

        test();test();
        IF( (sub(cnt_used,N_BASE_BANDS) >= 0) && (sub(unbits_L2,N_BITS_FILL_L2)>=0) && (sub(cnt_unused,1)>0) )
        {
          /* read from the bitstream */
          bptpt = pBst_L2 + sub(N_BITS_AVQ_L2, unbits_L2);        move16();
          ind_corr_max = GetBit( &bptpt, N_BITS_FILL_L2 ); 
          unbits_L2 = sub(unbits_L2, N_BITS_FILL_L2);

          IF( sub(ind_corr_max,CORR_RANGE_L2) < 0)
          {
            FOR( i=0; i<N_SV; i++ )
            {
              IF( zero_vector[i] == 0 )
              {
                IF( sub(k,CORR_RANGE_L1) < 0 )
                {
                  /*correct_rat = 1/Sqrt(sum_vect_E( &vec_base[ind_corr_max], WIDTH_BAND )/WIDTH_BAND);*/
                  L_en = Sum_vect_E( &svec_base[ind_corr_max], WIDTH_BAND );
                  L_tmp = norm_l_L_shl(&exp_den, L_en);
                  exp_den = sub(16, exp_den); /* 16 for round */
                  L_tmp = Isqrt_lc(L_tmp, &exp_den);
                  Gain16= extract_h_L_shr_sub(L_tmp, -6,exp_den);   /*Q15 */

                  IF( sub(Gain16, 32767)< 0 )
                  {
                    Gain16 = mult_r(Gain16,senv_BWE[i]);   /* Q15*Q12 ->Q12 */
                  }
                  ELSE
                  {
                    Gain16 = senv_BWE[i];    move16();    /* Q12 -> Q12 */
                  }
                  i8 = shl(i, 3);
                  FOR( j=0; j<WIDTH_BAND; j++ )
                  {
                    scoef_SWB_AVQ[i8+j] = round_fx_L_shl_L_mult(svec_base[ind_corr_max+j],Gain16,3);
                    move16(); 
                  }
                  zero_vector[i] = 2;                 move16();
                  BREAK;
                }
                k = 0;
              }
            }
          }
        }

        /*****************************/
#ifdef DYN_RAM_CNT
        DYN_RAM_POP();
#endif
        /*****************************/  
      }

      /*****************************/
#ifdef DYN_RAM_CNT
      DYN_RAM_POP();
#endif
      /*****************************/

      /*--------------------------------------------------------------------
      *
      * get sign information
      *
      *-------------------------------------------------------------------*/
        /* set pointer adress */
        if (sub (layers, 1) == 0)
        {
          unbits_L1 = sub (unbits_L1, N_BITS_FILL_L1+2);
          if (unbits_L1 < 0)
          {
            unbits_L1 = 0; move16 ();
          }
        }
        bptpt_L1 = pBst_L1 + sub(N_BITS_AVQ_L1, unbits_L1);	move16();

        if (sub (layers, 2) == 0)
        {
          bptpt_L2 = pBst_L2 + sub(N_BITS_AVQ_L2, unbits_L2);
        }

        /* allocate sign information */
        FOR (i=0; i<N_SV; i++)
        {
          IF (sub (zero_vector[i], 1) == 0)
          {
            i8 = shl (i, 3);
            FOR (j=0; j<WIDTH_BAND; j++)
            {
              IF (unbits_L1 > 0)
              {
                IF (stmp_coef_SWB_AVQ[i8+j] == 0)
                {
                  sbit = GetBit (&bptpt_L1, 1);

                  stmp = abs_s (scoef_SWB_AVQ[i8+j]);
                  if (sbit == 0)
                  {
                    stmp = negate (stmp);
                  }
                  scoef_SWB_AVQ[i8+j] = stmp; move16 ();

                  unbits_L1 = sub (unbits_L1, 1);
                }
              }
              ELSE IF (sub (layers, 2) == 0)
              {
                IF (unbits_L2 > 0)
                {
                  IF (stmp_coef_SWB_AVQ[i8+j] == 0)
                  {
                    sbit = GetBit (&bptpt_L2, 1);

                    stmp = abs_s (scoef_SWB_AVQ[i8+j]);
                    if (sbit == 0)
                    {
                      stmp = negate (stmp);
                    }
                    scoef_SWB_AVQ[i8+j] = stmp; move16 ();
                    unbits_L2 = sub (unbits_L2, 1);
                  }
                }
              }              
            }
          }
        }
      /*------------------------------------------------------------------*/
    }
    /* Read adjusted gain index */
    bptpt = pBst_L1 + N_BITS_AVQ_L1;
  }
  ELSE
  {
    /*----------------------------------------------------------------------
    *
    * AVQ without mode switching
    *
    *---------------------------------------------------------------------*/

    /* ***** decode SWBL1 and SWBL2 AVQ parameters ***** */
    decoder_SWBL1L2_AVQ( (void*)w_AVQ_state_dec, pBst_L1, pBst_L2, layers, senv_BWE, sord_b, zero_vector, scoef_SWB_AVQ 
      , 2, &unbits_L1, &unbits_L2);

    flg_bit = 2; move16();
    /* Read adjusted gain index */
    bptpt = pBst_L1 + N_BITS_AVQ_L1_PLS;
  }
  index_gain = GetBit( &bptpt, N_BITS_GAIN_SWBL1 );

  /* Obtain adjusted gain */
  sGainBWEQ = sub (14, index_g_5bit); /* Q(sGainBWEQ):14-index_g_5bit */

  test ();
  IF (index_g_5bit == 0 && sub(index_gain, 5) < 0)
  {
    sGopt = sg0[index_gain]; move16 (); /* Q14:14-index_g_5bit=14-0 */
  }
  ELSE
  {
    sGopt = sgain_frac[index_gain]; /* Q(sGainBWEQ) */
    move16();
  }

  /* for zero subbands, keep MDCT coeficients from the BWE SWBL0 */
  FOR( i = 0; i<N_SV; i++ )
  {
    i8 = shl(i, 3);
    IF( sub(zero_vector[i],1) == 0 )
    {
      /* apply adjusted global gain to AVQ decoded MDCT coeficients */
      tmp16 = add(15-12, sub(*scoef_SWBQ,sGainBWEQ));

      FOR( j = 0; j<WIDTH_BAND; j++ )
      {
        /*coef_SWB[j] = coef_SWB_AVQ[j]*fGopt;*/
        scoef_SWB[i8+j] = round_fx_L_shl_L_mult(scoef_SWB_AVQ[i8+j], sGopt,tmp16);
        move16();
      }
    }
    ELSE IF( sub(zero_vector[i],2) == 0 )
    {
      /* apply global gain to AVQ decoded MDCT coefficients */
      tmp16 = add(15-12, sub(*scoef_SWBQ,sGainBWEQ));

      FOR( j = 0; j<WIDTH_BAND; j++ )
      {
        /*coef_SWB[j] = coef_SWB_AVQ[j]*fGopt;*/
        scoef_SWB[i8+j] = round_fx_L_shl_L_mult(scoef_SWB_AVQ[i8+j], sGainBWE,tmp16);
        move16();
      }
    }									
    ELSE IF( sub(flg_bit,1 ) == 0)
    {
      /* apply global gain to AVQ decoded MDCT coefficients */
      tmp16 = add(15-12, sub(*scoef_SWBQ,sGainBWEQ));

      FOR( j = 0; j<WIDTH_BAND; j++ )
      {
        tmp16_2 = round_fx_L_shl_L_mult(senv_BWE[i], sGainBWE,tmp16);
        if( scoef_SWB[i8+j] < 0 )
        {
          tmp16_2 = negate(tmp16_2);      
        }
        scoef_SWB[i8+j] = tmp16_2;     move16();    
      }                
    }
    ELSE IF( sub(w_AVQ_state_dec->detzer_flg,1) == 0 )
    {
      Word16 Gain, exp_den, exp_num;
      Word32 L_tmp;

#ifdef DYN_RAM_CNT
      DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 + SIZE_Word32), "dummy");
#endif

      L_tmp = L_add(1L, L_mac_Array(WIDTH_BAND, &scoef_SWB[i8], &scoef_SWB[i8]));

      exp_den = norm_l(L_tmp);
      tmp16 = extract_h_L_shl(L_tmp, exp_den);
      exp_den = sub(30,exp_den);
      exp_den = sub(exp_den, shl(*scoef_SWBQ,1));
      Gain = 16384; move16();
      if (sub(16384, tmp16) <= 0)
      {
        Gain= div_s(16384, tmp16);
      }
      exp_num = sub(14,exp_den);

      L_tmp = Isqrt_lc(L_deposit_h(Gain),&exp_num);
      L_tmp = L_mls(L_tmp, 3277);
      Gain= round_fx_L_shl(L_tmp, sub(add(exp_num,*scoef_SWBQ),10));
      FOR( j = 0; j<WIDTH_BAND; j++ )
      {
        if( scoef_SWB[i8+j] < 0 )
        {
          Gain = negate(Gain);    
        }
        scoef_SWB[i8+j] = Gain;    move16();
      }
#ifdef DYN_RAM_CNT
      DYN_RAM_POP();
#endif  
    }
  }

  test();
  IF ( sub(flg_bit, 1) != 0 && sub(layers, 2) == 0 )
  {
    /* modify decoded MDCT coefs. using gradient */
    decoder_coef_SWB_AVQ_adj(zero_vector, sord_b, scoef_SWB, pBst_L1, &unbits_L1, 1, N_BITS_AVQ_L1);

    /* modify decoded MDCT coefs. using gradient */
    decoder_coef_SWB_AVQ_adj(zero_vector, sord_b, scoef_SWB, pBst_L2, &unbits_L2, 2, N_BITS_AVQ_L2);
  }

  scoef_SWBQ2 = Exp16Array(SWB_F_WIDTH, w_AVQ_state_dec->sbuffAVQ);
  array_oper(SWB_F_WIDTH, scoef_SWBQ2, w_AVQ_state_dec->sbuffAVQ, w_AVQ_state_dec->sbuffAVQ, &shl);

  scoef_SWBQ2 = add(scoef_SWBQ2, w_AVQ_state_dec->pre_scoef_SWBQ1);

  IF(sub(scoef_SWBQ2, *scoef_SWBQ) > 0)
  {
    scoef_SWBQ1 = *scoef_SWBQ;		move16();	
    temp = sub(scoef_SWBQ2, *scoef_SWBQ);
    array_oper(SWB_F_WIDTH, temp, w_AVQ_state_dec->sbuffAVQ, w_AVQ_state_dec->sbuffAVQ, &shr);

  }
  ELSE
  {
    scoef_SWBQ1 = scoef_SWBQ2;
    move16();
  }

  tmp16 = add(3, sub(scoef_SWBQ1, sGainBWEQ));
  tmp16_2 = sub(8, sGainBWEQ);
  FOR(i=0; i<N_SV; i++)
  {
    pos1 = shl(i, 3);
    pos2 = add(pos1, WIDTH_BAND);
    k = sGopt; move16();
    j = sub(zero_vector[i], 1);
    IF (j >= 0)
    {
      test();
      if (j > 0) /* sub(zero_vector[i], 1) > 0, so its value is 2 */
        k = sGainBWE; move16();

      FOR (j = pos1; j < pos2; j++)
      {
        w_AVQ_state_dec->sbuffAVQ[j] = round_fx_L_shl_L_mult(scoef_SWB_AVQ[j], k, tmp16);  /* Q(scoef_SWBQ1) */
        move16();
      }
    } 
    senv_avrg[i] = round_fx_L_shl_L_mult(senv_avrg[i], k, tmp16_2);   /* Q(5) */
    move16();
  }  

  test(); test();
  IF (sub((Word16) cod_Mode, HARMONIC) == 0 && sub(w_AVQ_state_dec->pre_cod_Mode, HARMONIC) == 0)
  {
    Word32 L_preAVQ1[SWB_F_WIDTH];
    Word32 L_prefSp[SWB_F_WIDTH];
    Word32 L_preAVQ0[SWB_F_WIDTH];

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3 * SWB_F_WIDTH) * SIZE_Word32), "dummy");
#endif
    /*****************************/

    IF(sub(w_AVQ_state_dec->pre_scoef_SWBQ0, w_AVQ_state_dec->pre_scoef_SWBQ1) > 0)
    {
      temp = sub(w_AVQ_state_dec->pre_scoef_SWBQ0, w_AVQ_state_dec->pre_scoef_SWBQ1);

      FOR(i=0; i<SWB_F_WIDTH; i++)
      {
        L_preAVQ1[i] = L_shl(L_deposit_l(w_AVQ_state_dec->spreAVQ1[i]), temp);	move32();	
        L_prefSp[i] = L_shl(L_deposit_l(w_AVQ_state_dec->sprefSp[i]), temp);	  move32();	
        L_preAVQ0[i] = L_deposit_l(w_AVQ_state_dec->spreAVQ0[i]);               move32();
      }
      scoef_SWBQ2 = w_AVQ_state_dec->pre_scoef_SWBQ0;	move16();	
    }
    ELSE
    {
      temp = sub(w_AVQ_state_dec->pre_scoef_SWBQ1, w_AVQ_state_dec->pre_scoef_SWBQ0);

      FOR(i=0; i<SWB_F_WIDTH; i++)
      {
        L_preAVQ1[i] = L_deposit_l(w_AVQ_state_dec->spreAVQ1[i]);                 move32();
        L_prefSp[i] = L_deposit_l(w_AVQ_state_dec->sprefSp[i]);                   move32();
        L_preAVQ0[i] = L_shl( L_deposit_l(w_AVQ_state_dec->spreAVQ0[i]), temp);		move32();	
      }
      scoef_SWBQ2 = w_AVQ_state_dec->pre_scoef_SWBQ1;		move16();
    }
    temp = sub(scoef_SWBQ1, scoef_SWBQ2);
    temp1 = sub(scoef_SWBQ1, *scoef_SWBQ);
    temp2 = add(15, temp);
    temp3 = add(15, temp1);
    FOR (i = 0; i < N_SV; i++)
    { 
      pos1 = shl(i, 3);
      test();
      IF ( w_AVQ_state_dec->prev_zero_vector[i] != 0 && zero_vector[i] == 0) 
      {
        FOR (j = 0; j < WIDTH_BAND; j++)
        { 
          k = add(pos1, j);
          lbuff_coef_pow = L_abs_L_deposit_l(scoef_SWB[k]); move32();
          IF (scoef_SWB_AVQ[k] == 0)
          { 
            IF(L_sub(L_preAVQ0[k], L_add(L_preAVQ1[k], L_prefSp[k])) > 0)
            {
              L_temp = L_add(L_mls(L_shl(L_preAVQ1[k], temp2), 3),
                L_mls(L_shl(L_prefSp[k], temp2), 3));
              L_temp = L_add(L_temp, L_mls(L_shl(lbuff_coef_pow, temp3), 2));
              en = round_fx_L_shl(L_temp, 13);                                         /* Q(scoef_SWBQ1) */
            }
            ELSE
            {
              L_temp = L_add( L_shl(L_preAVQ0[k], temp),
                L_mls(L_shl(L_preAVQ1[k], temp2), 3));
              L_temp = L_add(L_temp, L_mls(L_shl(L_prefSp[k], temp2), 3));
              L_temp = L_add(L_temp, L_shl(lbuff_coef_pow, temp1));
              en = round_fx_L_shl(L_temp, 13);                                           /* Q(scoef_SWBQ1) */
            }
            if_negate( &scoef_SWB[k], en);
          }
        }
      }
    }
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/  
  }
  ELSE IF(sub((Word16) cod_Mode, TRANSIENT) == 0 || sub( w_AVQ_state_dec->pre_cod_Mode, TRANSIENT) == 0)
  {
    Word32 L_preAVQ1[SWB_F_WIDTH];

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (SWB_F_WIDTH * SIZE_Word32), "dummy");
#endif
    /*****************************/

    temp1 = sub(scoef_SWBQ1, 5);
    FOR (i = 0; i < N_SV; i++)
    {
      pos1 = shl(i, 3);
      IF(zero_vector[i] != 0)
      {
        FOR (j = 0; j < WIDTH_BAND; j++)
        {
          k = add(pos1, j);
          IF (scoef_SWB_AVQ[k] == 0)
          {
            lbuff_coef_pow = L_abs_L_deposit_l(scoef_SWB[k]); move32();
            temp = sub(15, *scoef_SWBQ);
            if (k == 0)
            {			
              L_preAVQ1[k+1] = L_abs_L_deposit_l(w_AVQ_state_dec->sbuffAVQ[k+1]); move32();
              L_temp = L_shl(L_preAVQ1[k+1], *scoef_SWBQ);
            }
            ELSE IF (sub(k, SWB_F_WIDTH - 1) != 0)
            { 
              L_preAVQ1[k-1] = L_abs_L_deposit_l(w_AVQ_state_dec->sbuffAVQ[k-1]); move32();	
              L_preAVQ1[k+1] = L_abs_L_deposit_l(w_AVQ_state_dec->sbuffAVQ[k+1]);	move32();
              L_temp = L_shl(L_preAVQ1[k-1], *scoef_SWBQ);
              L_temp = L_add(L_temp, L_shl(L_preAVQ1[k+1], *scoef_SWBQ));
              temp = sub(14, *scoef_SWBQ);
            } 
            L_temp = L_add(L_temp, L_shl(lbuff_coef_pow, scoef_SWBQ1));
            en = round_fx_L_shl(L_temp, temp);
            tmp16 = round_fx_L_shl_L_mult(senv_avrg[i], 19661, temp1);  /* Q(scoef_SWBQ1) */
            en = s_min(en, tmp16);

            if_negate( &scoef_SWB[k], en);
          }
        }
      }
    }  
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/  
  }
  ELSE    /* NORMAL */
  {  
    Word16 abs_scoef_SWB[SWB_F_WIDTH];
    Word16 sh_sprefSp[SWB_F_WIDTH];

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2 * SWB_F_WIDTH) * SIZE_Word16), "dummy");
#endif
    /*****************************/

    temp = sub(scoef_SWBQ1, w_AVQ_state_dec->pre_scoef_SWBQ1);
    temp1 = sub(scoef_SWBQ1, *scoef_SWBQ);
    temp2 = sub(scoef_SWBQ1, 5);
    temp3 = sub(*scoef_SWBQ, 5);
    if(sub((Word16) flg_bit, 1) == 0)
    {
      ptr0 = scoef_SWB;
      ptr1 = abs_scoef_SWB;
      FOR (j = 0; j < SWB_F_WIDTH; j++)
      {
        *ptr1++ = shl(abs_s(*ptr0++), temp1);  move16();  
      }
      ptr0 = w_AVQ_state_dec->sprefSp;
      ptr1 = sh_sprefSp;
      array_oper(SWB_F_WIDTH, temp, ptr0, ptr1, &shl);
      FOR (i = 0; i < N_SV; i++)
      {
        pos1 = shl(i, 3);
        IF (zero_vector[i] == 0)
        { 
          tmp16 = round_fx_L_shl_L_mult(senv_avrg[i], 19661, temp2);  /* Q(scoef_SWBQ1) */
          FOR (j = 0; j < WIDTH_BAND; j++)
          {  
            k = add(pos1, j);
            IF (k == 0)
            { 
              L_temp = L_add( L_mult0(sh_sprefSp[k], 22938),
                L_mult0(abs_scoef_SWB[k], 29491));
              L_temp = L_add(L_temp, L_mult0(sh_sprefSp[k+1], 6554));
              L_temp = L_add(L_temp, L_mult0(abs_scoef_SWB[k+1], 6554));
            }
            ELSE IF (sub(k, N_SV * WIDTH_BAND - 1) == 0)
            {
              L_temp = L_add( L_mult0(sh_sprefSp[k], 16384), L_mult0(abs_scoef_SWB[k], 22938));
              L_temp = L_add(L_temp, L_mult0(sh_sprefSp[k-1], 9830));
              L_temp = L_add(L_temp, L_mult0(shl(abs_s(scoef_SWB[k-1]), temp1), 16384));
            }
            ELSE
            { 
              L_temp = L_add( L_mult0(sh_sprefSp[k], 16384), L_mult0(abs_scoef_SWB[k], 22938));
              L_temp = L_add(L_temp, L_mult0(sh_sprefSp[k-1], 6554));
              L_temp = L_add(L_temp, L_mult0(shl(abs_s(scoef_SWB[k-1]), temp1), 6554));
              L_temp = L_add(L_temp, L_mult0(sh_sprefSp[k+1], 6554));
              L_temp = L_add(L_temp, L_mult0(abs_scoef_SWB[k+1], 6554));
            } 
            en = round_fx(L_temp); 
            en = s_min(en, tmp16);  
            if_negate( &scoef_SWB[k], en);
          }  
        }
        ELSE
        {
          tmp16 = round_fx_L_shl_L_mult(senv_avrg[i], 19661, temp3);  /* Q(scoef_SWBQ) */
          FOR (j = 0; j < WIDTH_BAND; j++)
          {
            k = add(pos1, j);
            IF (w_AVQ_state_dec->sbuffAVQ[k] == 0)
            {
              IF (k == 0)
              {
                L_temp = L_add(L_mult(sh_sprefSp[k], 4915), L_mult(abs_scoef_SWB[k], 21299));
                L_temp = L_add(L_temp, L_mult(sh_sprefSp[k+1], 3277));
                L_temp = L_add(L_temp, L_mult(abs_scoef_SWB[k+1], 3277));
              }
              ELSE IF (sub(k, N_SV * WIDTH_BAND - 1) == 0)
              {
                L_temp = L_add(L_mult(sh_sprefSp[k], 4915), L_mult(abs_scoef_SWB[k], 21299));
                L_temp = L_add(L_temp, L_mult(sh_sprefSp[k-1], 1638));
                L_temp = L_add(L_temp, L_mult(shl(abs_s(scoef_SWB[k-1]), temp1), 4915));
              }
              ELSE
              {
                L_temp = L_add(L_mult(sh_sprefSp[k], 4915), L_mult(abs_scoef_SWB[k], 21299));
                L_temp = L_add(L_temp, L_mult(sh_sprefSp[k-1], 1638));
                L_temp = L_add(L_temp, L_mult(shl(abs_s(scoef_SWB[k-1]), temp1), 1638));
                L_temp = L_add(L_temp, L_mult(sh_sprefSp[k+1], 1638));

                L_temp = L_add(L_temp, L_mult(abs_scoef_SWB[k+1], 1638));
              }
              en = round_fx(L_temp);
              en = s_min(en, tmp16);         
              if_negate( &scoef_SWB[k], en);
            }
            ELSE
            {
              scoef_SWB[k] = shl(scoef_SWB[k], temp1);	move16();	
            }
          }
        }
      }
    }
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/  
  }

  mov16(N_SV, zero_vector, w_AVQ_state_dec->prev_zero_vector);

  mov16(SWB_F_WIDTH, w_AVQ_state_dec->spreAVQ1, w_AVQ_state_dec->spreAVQ0);
  abs_array(w_AVQ_state_dec->sbuffAVQ, w_AVQ_state_dec->spreAVQ1, SWB_F_WIDTH);
  abs_array(scoef_SWB, w_AVQ_state_dec->sprefSp, SWB_F_WIDTH);
  w_AVQ_state_dec->pre_scoef_SWBQ0 = w_AVQ_state_dec->pre_scoef_SWBQ1; move16();

  *scoef_SWBQ = scoef_SWBQ1;			move16();	
  w_AVQ_state_dec->pre_scoef_SWBQ1 = *scoef_SWBQ;		move16();	

  IF (sub(flg_bit, 1) == 0)
  {
    /* ------------------------------------------------------------------ */
    /* smoothing (if decoding mode is 1)                                  */
    /* ------------------------------------------------------------------ */
    Word16 sbuff_abs; /* Q(scoef_SWBQ) */
    Word16 scoef_tmp; /* Q(scoef_SWBQ) */
    Word16 tmpQ;

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16), "dummy");
#endif
    /*****************************/ 

    /* for zero subbands, keep MDCT coeficients from the BWE SWBL0 */

    /* Adjust Q-value of previous frame to that of current frame */
    tmpQ = sub(w_AVQ_state_dec->scoef_SWB_abs_oldQ, *scoef_SWBQ);
    array_oper(SWB_F_WIDTH, tmpQ, w_AVQ_state_dec->scoef_SWB_abs_old, w_AVQ_state_dec->scoef_SWB_abs_old, &shr);

    w_AVQ_state_dec->scoef_SWB_abs_oldQ = *scoef_SWBQ;    move16();

    FOR ( i = 0; i<N_SV; i++ )
    {
      i8 = shl(i, 3);
      FOR ( j = 0; j<WIDTH_BAND; j++ )
      {
        sbuff_abs = abs_s(scoef_SWB[i8+j]);
        scoef_tmp = extract_h( L_add( L_mult(27853, sbuff_abs), L_mult(4915, w_AVQ_state_dec->scoef_SWB_abs_old[i8+j]) ) ); /* Q(scoef_SWBQ): (Q(15) + Q(scoef_SWBQ) + 1) - 16 */

        if (scoef_SWB[i8+j] < 0)
        {
          scoef_tmp = negate(scoef_tmp);
        }
        if( w_AVQ_state_dec->scoef_SWB_abs_old[i8+j] != 0 )
        {
          scoef_SWB[i8+j] = scoef_tmp;
          move16();
        }
        w_AVQ_state_dec->scoef_SWB_abs_old[i8+j] = sbuff_abs;
        move16();
      }
    }
    w_AVQ_state_dec->scoef_SWB_abs_oldQ = *scoef_SWBQ;
    move16();

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  ELSE
  {
    /* initialize */
    zero16(SWB_F_WIDTH, w_AVQ_state_dec->scoef_SWB_abs_old);

    w_AVQ_state_dec->scoef_SWB_abs_oldQ = 0;
    move16();
  }
  w_AVQ_state_dec->pre_cod_Mode = cod_Mode;
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

/*-----------------------------------------------------------------*
*   Funtion  decoder_coef_SWB_AVQ_adj                             *
*            ~~~~~~~~~~~~~~~~~~~~~~~~                             *
*   calculate gradient of regression line                         *
*-----------------------------------------------------------------*/
static void decoder_coef_SWB_AVQ_adj( 
                                     const Word16   zero_vector[],  /* o:  output vector signalising zero bands     */
                                     const Word16   sord_b[],       /* i:  percept. importance  order of subbands   */
                                     Word16   coef_SWB[],     /* i/o:  locally decoded MDCT coefs.             */ /* Q(scoef_SWBQ) */
                                     UWord16  *pBst,           /* i/o:  pointer to bitstream buffer            */
                                     Word16  *unbits,
                                     const Word16   smode,          /* 1: L1 / 2: L2                                */
                                     const Word16   N_BITS_AVQ
                                     )
{
  Word16 i, j, b, n;
  Word16 idx, max_idx, bit_alloc;
  Word16 i8, id8;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (9 * SIZE_Word16), "dummy");
#endif
  /*****************************/

  /* pointer of bitstream */
  pBst = pBst + N_BITS_AVQ - (*unbits);

  /* calculate the number of vector */
  n = 0; move16();
  FOR (i=0; i<N_SV; i++)
  {
    if (sub(zero_vector[i], smode) == 0)
    {
      n = add(n,1);
    }
  }

  /* calculate gradient */
  /* ------------------ */
  FOR (i=0; i<N_SV; i++)
  {
    /* check unbits */
    if (!*unbits)
    {
      BREAK;
    }

    /* calculate gradient of each vector */
    b = sord_b[i]; move16();
    IF (sub(zero_vector[b], smode) == 0) 
    {
      /* calculate bit allocation */
      max_idx = 1; move16();
      bit_alloc = 1; move16();
      IF (sub(*unbits, n) > 0)
      {
        max_idx = 3;
        move16();
        bit_alloc = 2;
        move16();
      }

      n = sub(n,1);

      /* read from the bitstream */                    
      idx = GetBit(&pBst, bit_alloc);
      *unbits = sub( *unbits, bit_alloc);

      /* update locally decoded MDCT coefs. */
      IF (idx)
      { 
        i8 = shl(b, 3); 
        id8 = shl( sub(idx,1), 3);

        FOR (j=0; j<WIDTH_BAND; j++)
        {
          coef_SWB[i8+j] = shl( mult(coef_SWB[i8+j], sgrad[id8+j]), 1 ); /* Q(scoef_SWBQ): Q(scoef_SWBQ) + Q(14) + 1 - 16 + 1 */ 
          move16();
        }

      }     
    } /* if (zero_vector[b] == mode) */
  } /* for (i=0; i<N_SV; i++) */

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
}
