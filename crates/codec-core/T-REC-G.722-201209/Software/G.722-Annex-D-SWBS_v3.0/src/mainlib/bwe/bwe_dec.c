/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "bit_op.h"
#include "bwe_mdct.h"
#include "bwe.h"
#include "softbit.h"
#include "table.h"
#include "math_op.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#define DECODER_OK  2
#define DECODER_NG  3

void proc_Env(Word16 * temp, Word16 senerWB, Word16 i_max, Word16 *sfEnv, Word16 *coef_SWBQ1, Word16 *coef_SWBQ2, Word16 *scoef_SWBQ, 
              Word16 *pre_sfEnv, Word16 i, Word16 * temp1, Word16 * temp2, BWE_state_dec *dec_st, Word16 norm_MDCT_fix1, Word16 a, Word16 b)
{
  *temp = div_s(senerWB, i_max); /* Q(13) */
  *temp = mult_r(*temp, 16384);  /* Q(15) */
  FOR(i=0; i<8; i++)
  {
    sfEnv[i] = mult_r(*temp, sfEnv[i]); /* Q(norm_MDCT_fix1) */
    move16(); 
  }

  *coef_SWBQ1 = Exp16Array(8, sfEnv);
  *coef_SWBQ2 = Exp16Array(8, pre_sfEnv);
  FOR(i=0; i<8; i++)
  {
    sfEnv[i] = mult_r(shl(sfEnv[i], *coef_SWBQ1), a);          /* Q(norm_MDCT_fix1+coef_SWBQ1) */
    pre_sfEnv[i] = mult_r(shl(pre_sfEnv[i], *coef_SWBQ2), b);  /* Q(pre_coef_SWBQ+coef_SWBQ2) */
    move16();
    move16();
  }

  *temp1 = add(norm_MDCT_fix1, *coef_SWBQ1);
  *temp2 = add(dec_st->pre_coef_SWBQ, *coef_SWBQ2);
  *temp = sub(*temp1, *temp2);
  IF(*temp > 0)
  {
    *scoef_SWBQ = *temp2;  move16();
    FOR(i=0; i<8; i++)
    {
      sfEnv[i] = add(shr(sfEnv[i], *temp), pre_sfEnv[i]); move16();
    }
  }
  ELSE
  {
    *temp = sub(*temp2, *temp1);
    *scoef_SWBQ = *temp1;  move16();
    FOR(i=0; i<8; i++)
    {
      pre_sfEnv[i] = shr(pre_sfEnv[i], *temp);     move16();
      sfEnv[i] = add(sfEnv[i], pre_sfEnv[i]);      move16();
    }
  }
}

static Word16 getShift (Word16 *val, Word16 thres) 
{ 
  Word16 Shift2; 

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32)(SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif 
  Shift2 = 0; move16(); 
  IF(*val!= 0) 
  { 
    Shift2  = sub(norm_s(thres), norm_s(*val)); 
    IF(Shift2>= 0) 
    { 
      if(sub(shr(*val, Shift2), thres) > 0) 
      { 
        Shift2= add(Shift2,1); 
      } 
    } 
    Shift2 = s_max(0, Shift2); 
    *val= shr(*val, Shift2); 
  } 
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(Shift2); 
} 

Word32 L_mac0_Array_w(
                      Word16     sw,       /* I : initial weight*/
                      Word16     sfw,      /* I : weight step */
                      Word16     j0,       /* I : */
                      Word16     j1,       /* I : */
                      Word32     L_count,  /* I : intial counter */    
                      Word16     i_s,      /* I : index step */
                      Word16     *x        /* I : array */
                      )
{                            
  Word16 j;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32)(SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  FOR(j=j0; j != j1; j+=i_s)
  {
    L_count = L_mac0(L_count, sw, x[j]);  
    sw = sub(sw, sfw);
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return L_count;
}

void* bwe_decode_const(void)
{
  BWE_state_dec  *dec_st=NULL;

  dec_st = (BWE_state_dec *)malloc( sizeof(BWE_state_dec) );
  if (dec_st == NULL) return NULL;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;

    ssize = (UWord32) SIZE_Ptr;
#ifdef MEM_STT
    ssize += (UWord32)(sizeof(BWE_state_dec));
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  bwe_decode_reset( (void *)dec_st );

  return (void *)dec_st;
}

void bwe_decode_dest( void *work ) 
{
  BWE_state_dec  *dec_st=(BWE_state_dec *)work;


  if (dec_st != NULL)
  {
    free( dec_st );
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
}

Word16 bwe_decode_reset( void *work ) 
{
  BWE_state_dec  *dec_st=(BWE_state_dec *)work;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (SIZE_Ptr);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  IF (dec_st != NULL) {
    zero16(sizeof(BWE_state_dec)/2, (Word16*)dec_st);
    dec_st->sattenu2 = 3277;  move16();
    dec_st->pre_coef_SWBQ = 15;  move16();

    dec_st->Seed = 21211L; move32();
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return DECODER_OK;
}

Word32 bwe_dec_update( /*to maintain mid-band post-processing memories up to date in case of WB frame*/
                      Word16 *y_low,          /* (i): Input lower-band WB signal */
                      void   *work            /* (i/o): Pointer to work space        */
                      )
{
  Word16 sPrev[L_FRAME_WB], sY[L_FRAME_WB];
  Word16 norm;

  BWE_state_dec *dec_st = (BWE_state_dec *) work;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32)(SIZE_Ptr);
    ssize += (UWord32)((2 * L_FRAME_WB + 1)*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif    

  mov16(L_FRAME_WB, dec_st->pre_wb, sPrev);
  mov16(L_FRAME_WB, y_low, dec_st->pre_wb);


  bwe_mdct (sPrev, y_low, sY, &norm);
  norm = sub(norm, 1);
  PCMSWB_TDAC_inv_mdct (y_low, sY, dec_st->sPrev_wb, norm, 
    &dec_st->norm_pre_wb, (Word16) 0, dec_st->sCurSave_wb);

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return(0);
}

void MaskingFreqPostprocess(                     /* Generating the frequency gains with masking effect */
                            Word16 *spFreq,      /* (i) : Frequency coefficients */
                            Word16 Len,          /* (i) : Length of frequency coefficients */                              
                            Word16 *sgain,       /* (o) : Gains for each frequency */
                            Word16 sControl,     /* (i) : Control the degree of postprocessing : 0<Control<1 ;*/
                            Word16 Shift               /* Control=0 means no postprocessing */
                            )
{   
  Word32 L_temp;
  Word16 sFq_abs[L_FRAME_WB - ZERO_SWB], sMax_g;   
  Word16 Shift1;
  Word16 temp, temp1, temp2;

  Word16 sAverageMag, sMenergy, sMenergy_th, sNorm;
  Word16 i;
  Word16 Shift2;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) ((L_FRAME_WB - ZERO_SWB + 10)*SIZE_Word16);
    ssize += (UWord32) (SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  /* Calc average magnitude */
  abs_array(spFreq, sFq_abs,  Len);

  L_temp = L_mac0(0, sFq_abs[0], 1);
  FOR(i=1; i<Len; i++)
  {
    L_temp = L_mac0(L_temp, sFq_abs[i], 1);
  }
  L_temp = L_mls(L_temp, 29127);            /* Q(Shift + 5) */                          
  sAverageMag = round_fx_L_shl(L_temp, 11); /* AverageMag : Q(Shift) */

  FOR (i = 0; i < Len; i++)
  {
    /* Estimate Masked magnitude */
    L_temp = L_mac0_Array_w(8192, pst_fw1[i], 0, pst_j1[i] , 0, -1, &sFq_abs[i]);
    L_temp = L_mac0_Array_w(sub_8192_pst_fw2[i], pst_fw2[i], 1, pst_j2[i], L_temp, 1, &sFq_abs[i]);

    L_temp = L_mls(L_temp, pst_sumw1[i]);   /* Q(Shift+13+14-15) */

    sMenergy = round_fx_L_shl(L_temp, 4); 

    /* Estimate Masking threshold */
    L_temp = L_mac0_Array_w(16384, pst_fw11[i], 0, pst_j11[i], 0, -1, &sFq_abs[i]);
    L_temp = L_mac0_Array_w(sub_16384_pst_fw22[i], pst_fw22[i], 1, pst_j22[i], L_temp, 1, &sFq_abs[i]);

    L_temp = L_mls(L_temp, pst_sumw2[i]);

    sMenergy_th = round_fx_L_shl(L_temp, 1); 

    temp1 = mult_r(sMenergy_th, 24576);            /* Q(Shift) */
    temp2 = mult_r(sAverageMag, 8192);             /* Q(Shift) */
    sMenergy_th = add(temp1, temp2);               /* Q(Shift) */
    if(sMenergy_th == 0)
    {
      sMenergy_th = 1; move16();
    }
    Shift2 = getShift(&sMenergy, sMenergy_th);

    /*    Estimate gains  */
    sgain[i] = div_s(sMenergy, sMenergy_th);    move16(); /* gain[i]:Q(15-Shift2) */

    temp = sub(4, Shift2);

    sgain[i] = shr(sgain[i], temp);             move16();
  }

  /* Norm gains */
  L_temp = L_mac0(0, sgain[0], sFq_abs[0]);      /* Q(11+Shift) */
  sMax_g = s_max(0, sgain[0]);
  FOR(i=1; i<Len; i++)
  {
    L_temp = L_mac0(L_temp, sgain[i], sFq_abs[i]);      /* Q(11+Shift) */
    sMax_g = s_max(sMax_g, sgain[i]);
  }
  L_temp = L_mls(L_temp, 29127);                        /* Q(16+Shift) */
  sMenergy = round_fx(L_temp);                          /* Q(Shift) */

  Shift2 = 0; move16();
  IF(sMenergy > 0) 
  {
    Shift2 = getShift(&sAverageMag, sMenergy);
  }
  ELSE
  {
    sAverageMag = 0; move16();
    sMenergy = 1; move16();
  }

  sNorm = div_s(sAverageMag, sMenergy);                 /* Q(15-Shift2) */

  L_temp = L_mult(sMax_g, sNorm);                       /* Q(27-Shift2) */
  Shift1 = norm_l(L_temp);   
  temp = round_fx_L_shl(L_temp, Shift1);                /* Q(27-Shift2+Shift1-16) */
  Shift1 = sub(Shift1, Shift2);

  Shift1 = sub(Shift1, 2);
  temp = shr(temp, Shift1);                             /* Q(13) */
  IF(sub(temp, 12288) > 0)
  {
    temp1 = div_s(12288, temp);                         /* Q(15) */
    sNorm = mult_r(sNorm, temp1);                       /* Q(15-Shift2)   */
  }

  IF(L_msu0(L_shl(32, Shift), sAverageMag, 1) > 0)
  {   
    L_temp = L_shl(L_deposit_l(sControl), Shift); 
    L_temp = L_mac0(L_temp , sAverageMag, 512);
    L_temp = L_mls(L_temp, sNorm);                      /* Q(Shift+15-Shift2) */
    temp = sub(16, Shift);
    sNorm = round_fx_L_shl(L_temp, temp);               /* Q(15-Shift2)   */
  }

  Shift2 = add(Shift2, 3);
  FOR (i = 0; i < Len; i++) 
  {
    sgain[i] = round_fx_L_shl_L_mult(sgain[i], sNorm, Shift2);
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

void IF_Coef_Generator( Word16 *MDCT_wb_fix,   /* (i)input lower band MDCTs */
                       Word16 wb_Q,
                       Word16 mode,            /* (i)frame BWE mode */
                       Word16 *iSpectrum,      /* (o)SWB BWE MDCT coefficients */
                       Word16 *iEnv_fix,       /* (i)Frequency envelops */
                       Word16 *scoef_SWBQ,
                       Word16 sfGain,
                       Word16 *pre_sEnv,
                       Word16 pre_scoef_SWBQ,
                       Word16 pre_mode,
                       Word16 noise_flag,
                       Word16 bit_switch_flag,
                       BWE_state_dec* dec_st
                       )
{
  Word16 i, j;
  Word16 *pit_fix, *pit1_fix;
  Word32 iACC;
  Word16 iACC_hi, iACC_lo;
  Word32 norm_fix;

  Word16 if_Env_fix;
  Word16 weight_fix;
  Word16 temp;
  Word32 L_temp;
  Word32 L_temp1;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (2 * SIZE_Ptr);
    ssize += (UWord32) (7 * SIZE_Word16);
    ssize += (UWord32) (4 * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  temp = extract_l(dec_st->Seed); 
  test(); 
  IF (sub(noise_flag, 1) == 0 && sub(pre_mode, HARMONIC) != 0) 
  { 
    FOR (i = 0; i<SWB_F_WIDTH; i++) 
    { 
      L_temp = L_mac0(20101, temp, 12345);

      temp = extract_l(L_temp); 
      MDCT_wb_fix[i] = temp;     move16(); 
    } 
    wb_Q = 15; move16(); 
  } 
  ELSE 
  { 
    FOR (i = 0; i<SWB_F_WIDTH; i++) 
    { 
      L_temp = L_mac0(20101, temp, 12345);

      temp = extract_l(L_temp); 
    } 
    temp = add(mode, pre_mode ); 
    temp = add(temp, dec_st->modeCount); 
    IF (temp == 0) 
    { 
      mov16(SWB_F_WIDTH_HALF, &MDCT_wb_fix[SWB_F_WIDTH_HALF], MDCT_wb_fix); 
    } 
  }

  dec_st->Seed = L_add(L_temp, 0);

  IF(pre_scoef_SWBQ == 0) 
  { 
    Word16 norm; 
    pre_scoef_SWBQ = Exp16Array(8, pre_sEnv); 
    norm = pre_scoef_SWBQ; move16(); 
    if(sub(pre_scoef_SWBQ,15) > 0) 
    { 
      norm = 0; move16(); 
    } 
    pre_scoef_SWBQ = s_min(pre_scoef_SWBQ, 15); 

    array_oper8(norm, pre_sEnv, pre_sEnv, &shl);
  } 
  pit_fix = pre_sEnv; 
  temp = sub(*scoef_SWBQ, pre_scoef_SWBQ); 
  *scoef_SWBQ = s_min(*scoef_SWBQ, pre_scoef_SWBQ); 
  if (temp > 0) 
  {
    pit_fix = iEnv_fix; 
  } 

  array_oper8(abs_s(temp), pit_fix, pit_fix, &shr); 

  L_temp = L_shr( 42949673L, add(16, sub(sfGain, *scoef_SWBQ)));
  L_temp1= L_shr(1610612736L, sub(29, add(sfGain, *scoef_SWBQ)));  /* Q( 31- sfGain -2) --> *scoef_SWBQ */
  pit_fix = MDCT_wb_fix;
  pit1_fix = iSpectrum;
  temp = round_fx(L_temp);
  IF (sub(mode, TRANSIENT) == 0)
  {
    FOR (i = 0; i < SWB_TRANSI_FENV; i++)
    {
      iACC = L_mac0_Array(SWB_TRANSI_FENV_WIDTH, pit_fix, pit_fix);
      pit_fix += SWB_TRANSI_FENV_WIDTH;

      norm_fix = Inv_sqrt(iACC);     move16(); /* Max [ 8,---] Q( 30 - wb_Q -2)) */
      if (iEnv_fix[i] == 0) /* iEnv_fix */
      { 
        iEnv_fix[i] = temp;     move16();       
      } 
      pit_fix -= SWB_TRANSI_FENV_WIDTH;
      FOR (j = 0; j < SWB_TRANSI_FENV_WIDTH; j++)
      {
        iACC_lo = L_Extract_lc( norm_fix, &iACC_hi);
        iACC =Mpy_32_16( iACC_hi, iACC_lo,iEnv_fix[i] );
        iACC_lo = L_Extract_lc( iACC, &iACC_hi);

        iACC = Mpy_32_16( iACC_hi, iACC_lo, *pit_fix ); /* Q(wb_Q + scoef_SWBQ + 15) */
        *pit1_fix = extract_l(L_shl(iACC,2)); move16();
        pit_fix++;
        pit1_fix++;
      }
    }
  }
  ELSE
  {
    weight_fix = 16384; move16();          /* Q(15) */
    if(mode == 0)
    {
      weight_fix = 22937; move16();        /* Q(15) */
    }
    IF (s_and(bit_switch_flag, 1) == 0 )   /* test if bit_switch_flag == 0 or 2 */
    {
      FOR (i = 0; i < SWB_NORMAL_FENV; i++)
      {
        iACC = L_mac0_Array(FENV_WIDTH, pit_fix, pit_fix);
        pit_fix += FENV_WIDTH;

        iACC = L_shr(iACC,1);                    /* Q(tempx*2) */

        norm_fix = Inv_sqrt(iACC);     move32(); /* Max [ 8,---] Q( 30 - tempx-1) */

        temp = sub(iEnv_fix[i], pre_sEnv[i]);    /* scoef_SWBQ */
        L_temp = L_abs_L_deposit_l(temp);  
        if_Env_fix = iEnv_fix[i]; move16();
        test();
        IF(L_sub(L_temp, L_temp1) <= 0 && sub(pre_mode, TRANSIENT) != 0)
        {
          temp = sub((Word16)32767, weight_fix);
          iACC = L_mult(weight_fix, iEnv_fix[i]);

          if_Env_fix = mac_r(iACC,temp, pre_sEnv[i]);
        }
        pit_fix -= FENV_WIDTH;

        iACC_lo = L_Extract_lc(norm_fix,&iACC_hi);
        L_temp = Mpy_32_16( iACC_hi,iACC_lo,if_Env_fix);   /* 27 + scoef_SWBQ - 15 */
        iACC_lo = L_Extract_lc(L_temp,&iACC_hi);

        FOR(j=0; j<FENV_WIDTH; j++)
        {
          L_temp = Mpy_32_16( iACC_hi,iACC_lo,*pit_fix);   /* 27 + scoef_SWBQ - 15 + wb_Q - 15 */
          *pit1_fix = extract_l(L_shl(L_temp,1)); move16();
          pit_fix++;
          pit1_fix++;
        }
      }
    }
    ELSE
    {
      FOR (i = 0; i < SWB_NORMAL_FENV; i++)
      {
        iACC = L_mac0_Array(FENV_WIDTH, pit_fix, pit_fix);
        pit_fix += FENV_WIDTH;

        iACC = L_shr(iACC,1); /* Q(tempx*2) */
        norm_fix = Inv_sqrt(iACC);     move16(); /* Max [ 8,---] Q( 30 - tempx-1) */

        if_Env_fix = shr(add(iEnv_fix[i], pre_sEnv[i]),1); 
        pit_fix -= FENV_WIDTH;

        iACC_lo = L_Extract_lc(norm_fix,&iACC_hi);
        L_temp = Mpy_32_16( iACC_hi,iACC_lo,if_Env_fix);   
        iACC_lo = L_Extract_lc(L_temp,&iACC_hi);

        FOR(j=0; j<FENV_WIDTH; j++)
        {
          L_temp = Mpy_32_16( iACC_hi,iACC_lo,*pit_fix);   
          *pit1_fix = extract_l(L_shl(L_temp,1)); move16();
          pit_fix++;
          pit1_fix++;
        }
      }
    }
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

Word16 bwe_dec_freqcoef( UWord16 **pBit,             /* (i): Input bitstream                */
                        Word16  *y_low,              /* (i): Input lower-band WB signal */
                        void    *work,               /* (i/o): Pointer to work space        */
                        Word16  *sig_Mode,
                        Word16  *sTenv_SWB,          /* (o): Q(0) */ 
                        Word16  *scoef_SWB,          /* Q(scoef_SWBQ) */
                        Word16  *index_g,
                        Word16  *sFenv_SVQ,          /* (o): decoded spectral envelope with no postprocess. Q(scoef_SWBQ) */
                        Word16  ploss_status,
                        Word16  bit_switch_flag,
                        Word16  prev_bit_switch_flag,
                        Word16  *scoef_SWBQ
#ifdef LAYER_STEREO
                       ,Word16  channel
#endif
                        )
{
  BWE_state_dec *dec_st = (BWE_state_dec *) work;
  Word16 *spMDCT_wb;
  Word16 *spit_fen = scodebookL;
  Word16 i, j, mode;
  Word16 index_fGain = 0;
  Word16 index_fEnv[SWB_TRANSI_FENV];
  Word16 index_fEnv_codebook[NUM_FENV_CODEBOOK];
  Word16 index_fEnv_codeword[NUM_FENV_VECT];
  Word16 noise_flag = 0;
  Word16 norm;
  Word16 sY[L_FRAME_WB];
  Word16 T_modify_flag=0;
  Word16 MDCT_wb_fix[L_FRAME_WB], norm_MDCT_fix1;
  Word16 norm_MDCT_fix;
  Word16 Shift, temp, Shift1, temp1, temp2;
  Word16 pos, i_max, i_min, i_avrg;
  Word16 sfEnv[SWB_NORMAL_FENV], sNoExpand_fGain, senerL; 
  Word16 senerWB, coef_SWBQ1, coef_SWBQ2;
  Word16 spGain[36], sMDCT_wb_postprocess[L_FRAME_WB], senerL1, senerH;   
  Word16 pre_sfEnv[SWB_NORMAL_FENV];    
  Word16 sFenv_SVQQ;
  Word32 L_temp, L_temp1, L_temp2;
  Word32 sfGain;  

  zero16( SWB_TRANSI_FENV, index_fEnv);

  index_fEnv_codebook[0] = 0; move16();
  index_fEnv_codebook[1] = 0; move16();
  index_fEnv_codeword[0] = 0; move16();
  index_fEnv_codeword[1] = 0; move16();
  zero16( L_FRAME_WB, sY);
  zero16_8(sfEnv);

  zero16( L_FRAME_WB, sMDCT_wb_postprocess);
  move16(); move16(); move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (3 * SIZE_Ptr);
    ssize += (UWord32) ((26 + SWB_TRANSI_FENV + NUM_FENV_CODEBOOK + NUM_FENV_VECT + 3 * L_FRAME_WB + 2 * SWB_NORMAL_FENV) * SIZE_Word16);
    ssize += (UWord32) (4 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 
  mov16_8(dec_st->pre_fEnv, pre_sfEnv);

  IF (sub(ploss_status, 1) == 0)
  {
    /* MDCT */
    /* MDCT on 80 samples in the 0-8kHz band */
    bwe_mdct( dec_st->pre_wb, y_low, sY, &norm );
    array_oper(L_FRAME_WB, 18318, sY, MDCT_wb_fix, &mult);

    norm_MDCT_fix = sub(norm, 6);

    /* postprocess 4000-8000 Hz */
    spMDCT_wb = &MDCT_wb_fix[L_FRAME_WB - WB_POSTPROCESS_WIDTH];
    MaskingFreqPostprocess( spMDCT_wb, WB_POSTPROCESS_WIDTH, spGain, 16384, norm_MDCT_fix);

    IF (sub(dec_st->pre_mode, TRANSIENT) == 0)
    {
      mov16( WB_POSTPROCESS_WIDTH, spGain, dec_st->spGain_sm );
    }

    FOR (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
    {
      dec_st->spGain_sm[i] = extract_l_L_shr(L_mac0(L_deposit_l(dec_st->spGain_sm[i]), spGain[i], 1), 1); /* Q(14) */ 
      move16();
#ifdef LAYER_STEREO
      IF(sub(channel,1) == 0)
      {
#endif
      IF (sub(dec_st->spGain_sm[i], 18842) < 0)
      {
        spMDCT_wb[i] = round_fx_L_shl_L_mult(spMDCT_wb[i], dec_st->spGain_sm[i], 1); /* Q(norm_MDCT_fix) */
        move16();
      }
#ifdef LAYER_STEREO
      }
#endif

    } 
#ifdef LAYER_STEREO
    IF(sub(channel,1) == 0)
    {
#endif
    array_oper(L_FRAME_WB, 29309, MDCT_wb_fix, sMDCT_wb_postprocess, &mult);


    norm = Exp16Array(L_FRAME_WB,sMDCT_wb_postprocess);

    temp = sub(norm, 1);
    array_oper(L_FRAME_WB, temp, sMDCT_wb_postprocess, sY, &shl);

    norm = add(norm, add(norm_MDCT_fix, 3));

    PCMSWB_TDAC_inv_mdct (y_low, sY, dec_st->sPrev_wb, norm, 
      &dec_st->norm_pre_wb, (Word16) 0, dec_st->sCurSave_wb);
#ifdef LAYER_STEREO
    }
#endif
    /* copy the BWE parameters and decoded coefficients */
    *sig_Mode = NORMAL; move16();

    array_oper8(27853, dec_st->pre_fEnv, dec_st->pre_fEnv, &mult_r);

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif

    return (0);
  }
  ELSE
  {
    /* MDCT on 80 samples in the 0-8kHz band */
    bwe_mdct( dec_st->pre_wb, y_low, sY, &norm );

    array_oper(L_FRAME_WB, 18318, sY, MDCT_wb_fix, &mult);

    norm_MDCT_fix = sub(norm, 6);

    L_temp = L_mult0(MDCT_wb_fix[0], MDCT_wb_fix[0]);  /* Q(2*norm_MDCT_fix1) */
    L_temp1 = L_mls(L_temp, 1456);                     /* Q(2*norm_MDCT_fix1 + 16 -15) */
    FOR(i=1; i<ENERGY_WB; i++)
    {
      L_temp = L_mult0(MDCT_wb_fix[i], MDCT_wb_fix[i]); /* Q(2*norm_MDCT_fix1) */
      L_temp = L_mls(L_temp, 1456);                     /* Q(2*norm_MDCT_fix1 + 16 -15) */
      L_temp1 = L_add(L_temp1, L_temp);
    }
    senerL = L_sqrt(L_temp1);                           /* Q(norm_MDCT_fix1) */
    IF (s_and(bit_switch_flag, 1) == 0)                 /* test if bit_switch_flag == 0 or 2 */
    {
      mode = GetBit(pBit, 2);
      index_fGain = GetBit(pBit, 5);

      sfGain = L_shl(0x1L,index_fGain);

      IF(sub(mode, TRANSIENT) == 0)  /* normal frame */
      {
        dec_st->modeCount = 0; move16();
        L_temp = L_mls(L_shl(sfGain,15),6554); /* Q(15) */
        if( sub(index_fGain, 15) > 0)
          L_temp = L_mls(sfGain,6554); /* Q(15) */

        Shift = norm_l(L_temp);
        sNoExpand_fGain = round_fx_L_shl(L_temp, Shift);  /* Q(Shift) */

        index_fEnv[0] = GetBit(pBit, 4);
        move16();
        i_max = index_fEnv[0];  move16();
        pos = 0;  move16();
        FOR (i=1; i<VQ_FENV_DIM; i++)
        {
          index_fEnv[i] = GetBit(pBit, 4); move16();
          if(sub(index_fEnv[i], i_max) > 0)
          {
            pos = i;  move16();
          }
          i_max = s_max(index_fEnv[i], i_max); 
        }

        L_temp = L_mult0(index_fEnv[pos], sNoExpand_fGain);  /* Q(Shift) */
        Shift1 = norm_l(L_temp);

        FOR (i=0; i<VQ_FENV_DIM; i++)
        {
          L_temp = L_mult0(index_fEnv[i], sNoExpand_fGain);  /* Q(Shift) */
          sfEnv[i] = round_fx_L_shl(L_temp, Shift1);
          move16();
        }
        *scoef_SWBQ = sub(add(Shift1, Shift), 17);

        mov16_8(sfEnv, sFenv_SVQ);

        sFenv_SVQQ = *scoef_SWBQ; move16();
      }
      ELSE
      {
        index_fEnv_codebook[0] = GetBit1(pBit); move16();
        index_fEnv_codebook[1] = GetBit1(pBit); move16();

        index_fEnv_codeword[0] = GetBit(pBit, 6); move16();
        index_fEnv_codeword[1] = GetBit(pBit, 6); move16();
        spit_fen = CodeBookH;
        if (index_fEnv_codebook[0] == 0)
        {
          spit_fen = scodebookL;
        }

        temp = shl(index_fEnv_codeword[0], 2);
        mov16(SWB_NORMAL_FENV_HALF, &spit_fen[temp], sfEnv);

        spit_fen = CodeBookH;
        if (index_fEnv_codebook[1] == 0)
        {
          spit_fen = scodebookL;
        }
        temp = shl(index_fEnv_codeword[1], 2);

        mov16(SWB_NORMAL_FENV_HALF, &spit_fen[temp], &sfEnv[SWB_NORMAL_FENV_HALF]);

        Shift = Exp16Array(SWB_NORMAL_FENV,sfEnv);

        array_oper8(Shift, sfEnv, sfEnv, &shl);

        *scoef_SWBQ = add(12, Shift);
        *scoef_SWBQ = sub(*scoef_SWBQ, index_fGain);

        mov16_8(sfEnv, sFenv_SVQ);

        sFenv_SVQQ = *scoef_SWBQ;

        IF(sub(mode, HARMONIC) == 0)
        {
          dec_st->modeCount = add(dec_st->modeCount, 1);
        }
        ELSE
        {
          if(dec_st->modeCount > 0)
          {
            dec_st->modeCount = sub(dec_st->modeCount, 1);
          }
          noise_flag = 1; move16();
          if(mode == 0)
          {
            noise_flag = 0; move16();
          }
          mode = NORMAL; move16();

          i_max = s_max(0, sfEnv[0]); 
          i_min = s_min(shl(10, *scoef_SWBQ), sfEnv[0]);
          L_temp = L_mac0(0, sfEnv[0], 1);
          FOR(i=1; i<SWB_NORMAL_FENV; i++)
          {
            i_max = s_max(i_max, sfEnv[i]); 
            i_min = s_min(i_min, sfEnv[i]);
            L_temp = L_mac0(L_temp, sfEnv[i], 1);
          }
          i_avrg = round_fx_L_shl(L_temp, 13);     /* Q(scoef_SWBQ) */

          test();
          IF(sub(shl(sub(i_max, i_min), 1), shl(5, *scoef_SWBQ)) > 0 && sub(i_min, shl(12, *scoef_SWBQ)))
          {
            L_temp = L_mult0(2, i_avrg);
            FOR(i=0; i<SWB_NORMAL_FENV; i++)
            {
              IF(L_msu0(L_temp, 5, sfEnv[i]) > 0)
              {
                sfEnv[i] = mult_r(sfEnv[i], 16384); /* Q(*scoef_SWBQ) */
                move16();
              }
            }
          }
        }
      }

    }
    ELSE              
    {
      spMDCT_wb = &MDCT_wb_fix[20]; move16();
      FOR_L_mult_L_shr_L_add(4, spMDCT_wb, 3, &L_temp1, &L_temp);
      spMDCT_wb = spMDCT_wb + 4;
      sfEnv[0] = L_sqrt(L_temp1);                           /* Q(norm_MDCT_fix) */
      move16();

      L_temp2 = 0;    move32();

      spMDCT_wb = &MDCT_wb_fix[24];
      FOR(i=1; i<3; i++)
      {
        FOR_L_mult_L_shr_L_add(8, spMDCT_wb, 3, &L_temp1, &L_temp);
        spMDCT_wb = spMDCT_wb + 8;
        sfEnv[i] = L_sqrt(L_temp1);           move16();     /* Q(norm_MDCT_fix) */
      }
      FOR(i=3; i<8; i++)
      {
        FOR_L_mult_L_shr_L_add(8, spMDCT_wb, 3, &L_temp1, &L_temp);
        spMDCT_wb = spMDCT_wb + 8;
        sfEnv[i] = L_sqrt(L_temp1);           move16();     /* Q(norm_MDCT_fix) */

        L_temp = L_mls(L_temp1, 410);                       /* Q(2*norm_MDCT_fix-3) */
        L_temp2 = L_add(L_temp2, L_temp);
      }
      senerWB = L_sqrt(L_temp2);

      coef_SWBQ1 = Exp16Array(8, sfEnv);
      norm_MDCT_fix1 = norm_MDCT_fix; move16();
      test(); test();
      IF((coef_SWBQ1 > 0) || (coef_SWBQ1 == 0 && senerWB == 0))
      {
        IF(norm_MDCT_fix < 0)
        {
          array_oper8(norm_MDCT_fix, sfEnv, sfEnv, &shr);

          norm_MDCT_fix1 = 0; move16();
        }
      }

      i_max = MaxArray(8, sfEnv, &i);
      if(i_max == 0)
      {
        i_max = 1; move16();
      }
      if(prev_bit_switch_flag == 0)
      {
        dec_st->sattenu2 = 3277; move16();                           /* Q(15) */
      }

      {
        Word16 temp3 = 16384;
        Word16 temp4 = 16384;
        move16();move16();

#ifdef DYN_RAM_CNT
        DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16), "dummy");
#endif
        IF(sub(dec_st->sattenu2, 16384) < 0)
        {

          test();
          IF(sub(dec_st->prev_senerL, mult_r(senerL, 16384)) > 0 && sub(mult_r(dec_st->prev_senerL, 16384), senerL) < 0)
          {
            temp3 = dec_st->sattenu2; move16();
            temp4 =  sub(32767, dec_st->sattenu2);
          }
          dec_st->sattenu2 = add(dec_st->sattenu2, 328);

        }
        proc_Env(&temp, senerWB, i_max, sfEnv, &coef_SWBQ1, &coef_SWBQ2, scoef_SWBQ, pre_sfEnv, i, &temp1, &temp2, dec_st, norm_MDCT_fix1, temp3, temp4);
#ifdef DYN_RAM_CNT
        DYN_RAM_POP();
#endif
      }

      mode = NORMAL; move16();

      mov16_8(sfEnv, sFenv_SVQ);

      sFenv_SVQQ = *scoef_SWBQ; move16();
    }
    /* postprocess 4000-8000 Hz */
    temp = shr(MDCT_wb_fix[0], 2);
    L_temp = L_mac0(0, temp, temp); /* Q(2*norm_MDCT_fix) */
    FOR (i = 1; i < L_FRAME_WB; i++)
    {
      temp = shr(MDCT_wb_fix[i], 2);
      L_temp = L_mac0(L_temp, temp, temp); /* Q(2*norm_MDCT_fix) */

    }
    L_temp = L_mls(L_temp, 13107);         /* Q(2*norm_MDCT_fix) */
    senerL1 = L_sqrt(L_temp);              /* Q(norm_MDCT_fix) */
    L_temp = L_add_Array(SWB_NORMAL_FENV, sfEnv);

    senerH = extract_l_L_shr(L_temp, 3);   /* Q(*scoef_SWBQ) */

    spMDCT_wb = &MDCT_wb_fix[L_FRAME_WB - WB_POSTPROCESS_WIDTH];

    test();
    IF (sub(mode, TRANSIENT) != 0 && L_sub(L_shl(L_deposit_l(senerL1), *scoef_SWBQ), L_shl(mult_r(senerH, 26214), norm_MDCT_fix)) > 0)
    {
#ifdef LAYER_STEREO
      IF(sub(channel,2) == 0)
      {
         MaskingFreqPostprocess(spMDCT_wb, WB_POSTPROCESS_WIDTH, spGain, 22938, norm_MDCT_fix);
      }
      ELSE
      {
#endif
      MaskingFreqPostprocess(spMDCT_wb, WB_POSTPROCESS_WIDTH, spGain, 16384, norm_MDCT_fix);
#ifdef LAYER_STEREO
      }
#endif

      IF (sub(dec_st->pre_mode, TRANSIENT) == 0)
      {
        mov16(WB_POSTPROCESS_WIDTH, spGain, dec_st->spGain_sm);
      }
      FOR (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
      {
        dec_st->spGain_sm[i] = extract_l_L_shr(L_mac0(L_deposit_l(dec_st->spGain_sm[i]), spGain[i], 1), 1); /* Q(14) */ 

        move16();
#ifdef LAYER_STEREO
        IF(sub(channel,1) == 0)
        {
#endif
        IF (sub(dec_st->spGain_sm[i], 18842) < 0)
        {
          spMDCT_wb[i] = round_fx_L_shl_L_mult(spMDCT_wb[i], dec_st->spGain_sm[i], 1);  /* Q(norm_MDCT_fix) */
          move16();
        }
#ifdef LAYER_STEREO
        }
#endif
      }
    }
    ELSE
    {
      const16(WB_POSTPROCESS_WIDTH, 16384, dec_st->spGain_sm);
    }
#ifdef LAYER_STEREO
    IF(sub(channel,1) == 0)
    {
#endif
    array_oper(L_FRAME_WB, 29309, MDCT_wb_fix, sMDCT_wb_postprocess, &mult);
    norm = Exp16Array(L_FRAME_WB,sMDCT_wb_postprocess);
    temp = sub(norm, 1);
    array_oper(L_FRAME_WB, temp, sMDCT_wb_postprocess, sY, &shl);
    norm = add(norm, add(norm_MDCT_fix, 3));

    PCMSWB_TDAC_inv_mdct (y_low, sY, dec_st->sPrev_wb, norm, 
      &dec_st->norm_pre_wb, (Word16) 0, dec_st->sCurSave_wb);
#ifdef LAYER_STEREO
    }
#endif
    IF_Coef_Generator( MDCT_wb_fix, norm_MDCT_fix, mode, scoef_SWB, sfEnv, scoef_SWBQ, index_fGain, 
      dec_st->pre_fEnv, dec_st->pre_coef_SWBQ, dec_st->pre_mode, noise_flag, bit_switch_flag
      , dec_st);

    IF (sub(mode, TRANSIENT) == 0)
    {
      FOR (i=0; i<SWB_TENV; i++)
      {
        j = GetBit(pBit, 4);

        sTenv_SWB[i] = shl(1, j);
        move16();
      }

      T_modify_flag = GetBit1(pBit);
    }

    /* copy the BWE parameters and decoded coefficients */
    *sig_Mode = mode;  move16();

    mov16_8(sfEnv, dec_st->pre_fEnv);

    dec_st->pre_coef_SWBQ = *scoef_SWBQ; move16();
    dec_st->prev_senerL = senerL;  move16();

    *index_g = index_fGain; move16();
    temp = sub(sFenv_SVQQ, *scoef_SWBQ);

    array_oper8(temp, sFenv_SVQ, sFenv_SVQ, &shr_r);

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/

    return (T_modify_flag);
  } /*end of ploss_status!=1*/
}

void T_Env_Postprocess(Word16 *sOut, Word16 *tPre)
{
  Word16 i, temp;
  Word32 L_temp; 

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (2 * SIZE_Word16);
    ssize += (UWord32) (SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 
  L_temp = L_mult(sOut[0], 27852);
  temp = HALF_SUB_SWB_T_WIDTH_1;
  L_temp = L_mac(L_temp, tPre[temp], 2621);
  temp = HALF_SUB_SWB_T_WIDTH_2;
  L_temp = L_mac(L_temp, tPre[temp], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_3;
  sOut[0] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  L_temp = L_mult(sOut[1], 27852);
  L_temp = L_mac(L_temp, sOut[0], 2621);
  temp = HALF_SUB_SWB_T_WIDTH_1;
  L_temp = L_mac(L_temp, tPre[temp], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_2;

  sOut[1] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  L_temp = L_mult(sOut[2], 27852);
  L_temp = L_mac(L_temp, sOut[1], 2621);
  L_temp = L_mac(L_temp, sOut[0], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_1;

  sOut[2] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  FOR(i = 3; i < SWB_T_WIDTH; i++)
  {
    L_temp = L_mult(sOut[i], 27852);
    L_temp = L_mac(L_temp, sOut[i-1], 2621);
    L_temp = L_mac(L_temp, sOut[i-2], 1638);
    sOut[i] = mac_r(L_temp, sOut[i-3], 655);  /* Q(15) */
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return;
}

Word16 bwe_dec_timepos( Word16 sig_Mode,
                       Word16 *sTenv_SWB,  /* (i/o): Q(0) */
                       Word16 *scoef_SWB,
                       Word16 *y_hi,       /* (o): Output higher-band signal (Q0) */
                       void *work,         /* (i/o): Pointer to work space        */
                       Word16 erasure,
                       Word16 T_modify_flag,
                       Word16 *scoef_SWBQ
                       )
{
  BWE_state_dec *dec_st = (BWE_state_dec *)work;
  Word16 i, pos;
  Word16 sY[L_FRAME_WB], sOut[L_FRAME_WB];
  Word16 norm_MDCT = 0;

  Word16 iSpectrum_fix[SWB_T_WIDTH];

  Word16 *pit_fix;
  Word32  enn_fix;
  Word32  enn_fix1;
  Word16  enn_fix_hi;
  Word16  enn_fix_lo;

  Word16 max_env_fix, atteu_fix;

  Word32 ener_prev_fix;
  Word16 sPre_fix[HALF_SUB_SWB_T_WIDTH];

  Word16 norm_out;
  Word16 norm_out_pre;
  Word16 temp;

  move16();
  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (2 * SIZE_Ptr);
    ssize += (UWord32) ((10 + 2 * L_FRAME_WB + SWB_T_WIDTH + HALF_SUB_SWB_T_WIDTH) * SIZE_Word16);
    ssize += (UWord32) (3 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  IF (erasure == 0)
  {
    array_oper(SWB_F_WIDTH, 29308, scoef_SWB, iSpectrum_fix, &mult);
    zero16(ZERO_SWB, &sY[SWB_T_WIDTH-ZERO_SWB]);

    norm_MDCT = sub(Exp16Array(SWB_T_WIDTH-ZERO_SWB,iSpectrum_fix), 1);

    array_oper(sub(SWB_T_WIDTH, ZERO_SWB), norm_MDCT, iSpectrum_fix, sY, &shl);
    norm_MDCT = add(add(norm_MDCT, *scoef_SWBQ), 4);
  }

  PCMSWB_TDAC_inv_mdct (sOut, sY, dec_st->sPrev, norm_MDCT, 
    &dec_st->norm_pre, 
    (Word16) erasure,
    dec_st->sCurSave);

  /* temporal domain post-processing */   
  temp = sub(sig_Mode, TRANSIENT);
  test();
  IF ((temp == 0) || (sub(dec_st->pre_mode, TRANSIENT) == 0))
  {
    pit_fix = sOut; /* Q(6) */ 
    FOR (i = 0; i < SWB_TENV; i++)
    {
      enn_fix = L_mac0_Array(SWB_TENV_WIDTH, pit_fix , pit_fix );
      enn_fix = L_shr(enn_fix,4);

      enn_fix_lo = L_Extract_lc( enn_fix, &enn_fix_hi);

      enn_fix = Mpy_32_16( enn_fix_hi, enn_fix_lo, 26214 ); 
      enn_fix = Inv_sqrt(enn_fix); /* Q(30) */
      IF (temp != 0)
      {
        sTenv_SWB[i] = mult(dec_st->pre_tEnv, tEnv_weight[i]); move16(); /* Q(0) */
      }
      enn_fix1 = L_mls(enn_fix,sTenv_SWB[i]);
      FOR (pos = 0; pos < SWB_TENV_WIDTH; pos++)
      {
        *pit_fix = extract_l(L_mls(enn_fix1,*pit_fix)); move16();
        pit_fix++;
      }
    }
  }

  test();
  IF (temp == 0 && sub(T_modify_flag, 1) == 0)
  {
    max_env_fix = MaxArray(SWB_TENV, sTenv_SWB, &pos);
    pit_fix = sOut; 
    IF (pos != 0)
    {
      pit_fix += extract_l(L_mult0(SUB_SWB_T_WIDTH, pos));

      atteu_fix = div_s(sTenv_SWB[pos - 1] , sTenv_SWB[pos]); 
    }
    ELSE
    {
      ener_prev_fix = L_mac0_Array(HALF_SUB_SWB_T_WIDTH, dec_st->tPre, dec_st->tPre);
      ener_prev_fix = L_mls(ener_prev_fix,3277); /* divide 10 */
      SqrtI31(ener_prev_fix,&ener_prev_fix);
      atteu_fix = div_l( ener_prev_fix , sTenv_SWB[pos]);
    }
    array_oper(HALF_SUB_SWB_T_WIDTH, atteu_fix, pit_fix, pit_fix, &mult);
  }
  ELSE
  {
    mov16(HALF_SUB_SWB_T_WIDTH, dec_st->tPre, sPre_fix);

    norm_out_pre = Exp16Array(HALF_SUB_SWB_T_WIDTH, dec_st->tPre) ;
    array_oper(HALF_SUB_SWB_T_WIDTH, norm_out_pre, dec_st->tPre, sPre_fix, &shl); 

    norm_out = Exp16Array(L_FRAME_WB,sOut) ;
    array_oper(L_FRAME_WB, norm_out, sOut, sOut, &shl); 
    pit_fix = sOut;
    temp = sub(norm_out_pre, norm_out);
    i = L_FRAME_WB; move16();
    norm_out = s_min(norm_out_pre, norm_out);
    IF (temp >= 0)
    {
      norm_out_pre = temp; move16();
      pit_fix = sPre_fix;
      i = HALF_SUB_SWB_T_WIDTH; move16();
    }
    array_oper(i, abs_s(temp), pit_fix , pit_fix, &shr); 

    T_Env_Postprocess(sOut, sPre_fix);   
    array_oper(L_FRAME_WB, norm_out, sOut, sOut, &shr_r);
  }

  FOR(i=0; i<SWB_T_WIDTH; i+=2)
  {
    sOut[i] = negate(sOut[i]);   move16();
  }

  /* copy decoded sound data to output buffer */
  mov16(SWB_T_WIDTH, sOut, y_hi);
  mov16( HALF_SUB_SWB_T_WIDTH, &sOut[SWB_T_WIDTH-HALF_SUB_SWB_T_WIDTH], dec_st->tPre );
  if (sub(sig_Mode, TRANSIENT) == 0)
  {
    dec_st->pre_tEnv = sTenv_SWB[3];
    move16();
  }

  dec_st->pre_mode = sig_Mode;   move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return DECODER_OK;
}

#ifdef LAYER_STEREO
void T_Env_Postprocess_stereo(Word16 *sOut, Word16 *tPre)
{
  Word16 i, temp;
  Word32 L_temp; 

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (0 * SIZE_Ptr);
    ssize += (UWord32) (2 * SIZE_Word16);
    ssize += (UWord32) (1 * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  L_temp = L_mult(sOut[0], 27852);
  temp = HALF_SUB_SWB_T_WIDTH_1;
  L_temp = L_mac(L_temp, tPre[temp], 2621);
  temp = HALF_SUB_SWB_T_WIDTH_2;
  L_temp = L_mac(L_temp, tPre[temp], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_3;
  sOut[0] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  L_temp = L_mult(sOut[1], 27852);
  L_temp = L_mac(L_temp, sOut[0], 2621);
  temp = HALF_SUB_SWB_T_WIDTH_1;
  L_temp = L_mac(L_temp, tPre[temp], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_2;

  sOut[1] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  L_temp = L_mult(sOut[2], 27852);
  L_temp = L_mac(L_temp, sOut[1], 2621);
  L_temp = L_mac(L_temp, sOut[0], 1638);
  temp = HALF_SUB_SWB_T_WIDTH_1;

  sOut[2] = mac_r(L_temp, tPre[temp], 655);  /* Q(15) */
  move16();

  FOR(i = 3; i < SWB_T_WIDTH; i++)
  {
    L_temp = L_mult(sOut[i], 27852);
    L_temp = L_mac(L_temp, sOut[i-1], 2621);
    L_temp = L_mac(L_temp, sOut[i-2], 1638);
    sOut[i] = mac_r(L_temp, sOut[i-3], 655);  /* Q(15) */
    move16();
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

#endif

