/* ITU G.722 3rd Edition (2012-09) */

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

#define ENCODER_OK  0
#define ENCODER_NG  1

static void calc_avrg(Word16 n, Word16 *x, Word16 log_rms, Word16 *avrg_fix) 
{
  Word16 temp_fix, temp16_fix,  i;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (3*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  temp_fix = 3;  move16();
  FOR (i = 0; i < n; i++)
  {
    temp_fix = add(temp_fix, shr(x[i],3)); /* Q(11) -3 Q(8) */
  }
  temp16_fix = i_mult(log_rms, n); /* Q(11) -3 */

  if(sub(temp16_fix, shr(temp_fix, 1)) > 0)
  {
    *avrg_fix = 0;  move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

void calc_half_fenv(Word16 *spit, Word16 sgain_tmp, Word16 sfSpectrumQ, Word32 *Sphere, Word16 *sfEnv, Word16 *sfEnv_unq)
{
  Word32 L_temp, L_temp1;
  Word16 i, j, temp, Shift;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (4*SIZE_Word16);
    ssize += (UWord32) (2*SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/

  *Sphere = 0; move32();        
  FOR (i = 0; i < NORMAL_FENV_HALVE; i++)
  {
    L_temp = 0; move32();
    FOR (j = 0; j < FENV_WIDTH; j++)
    {
      L_temp1 = L_mult0(*spit, sgain_tmp);    /* Q(sfSpectrumQ+15) */
      Shift = norm_l(L_temp1);
      temp = round_fx_L_shl(L_temp1, Shift);  /* Q(sfSpectrumQ+15+Shift-16) */

      L_temp1 = L_mult0(temp, temp);          /* Q(2*(sfSpectrumQ+Shift-1)) */

      temp = add(sfSpectrumQ, Shift);
      temp = shl(temp, 1);
      temp = sub(24, temp);
      L_temp1 = L_shl(L_temp1, temp);         /* Q(25) */
      L_temp = L_add(L_temp, L_temp1);
      spit++;
    }

    *Sphere = L_add(*Sphere, L_temp);         /* Q(25) */			
    sfEnv[i] = L_sqrt(L_temp);  move16();     /* Q(12) */
    sfEnv_unq[i] = sfEnv[i];  move16();       /* Q(12) */
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}

/* temporal envelop calculation */
Word16
Icalc_tEnv( Word16 *sy,         /* (i/o)   current SWB high band signal  */ /* Q(0)  */
           Word16 *srms,       /* (o)    log2 of the temporal envelope  */ /* Q(11) */
           Word16 * transient,
           Word16 preMode
           , void* work    
           )

{
  Word16 i1;
  Word16  i, pos = 0;
  Word16 T_modify_flag = 0;
  Word16 log_rms_fix[NUM_FRAME * SWB_TENV]; /* Q(11) */ 	
  Word16 temp_fix, i_max_fix, max_deviation_fix; /* Q(11) */
  Word16 max_rise_fix;
  Word32  ener_total_fix;
  Word32  log2_tmp;
  Word16  log2_exp;
  Word16  log2_frac;
  Word32 temp32;
  Word16  gain_fix;
  Word16 *pit_fix;
  Word16 avrg_fix = 1;
  Word16 avrg1_fix = 1;
  Word32 ener_front_fix;
  Word32 ener_behind_fix;
  Word16 max_rms_fix;
  Word32 enerEnv = 0;
  BWE_state_enc* enc_st = (BWE_state_enc*)work;
  move16(); move16(); move16();move16();
  move32();

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (2*SIZE_Ptr);
    ssize += (UWord32) ((1+3+NUM_FRAME*SWB_TENV+3+1+6)*SIZE_Word16);
    ssize += (UWord32) (5*SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/     

  ener_total_fix = 0; move32();

  pit_fix = sy;

  mov16(NUM_PRE_SWB_TENV, enc_st->log_rms_fix_pre, log_rms_fix);

  ener_total_fix = L_add(enc_st->enerEnvPre[0], enc_st->enerEnvPre[1]);

  FOR (i = 0; i < SWB_TENV; i++)  /* 0 --- 4 */
  {
    temp32 = L_mac0_Array(SWB_TENV_WIDTH, &sy[i_mult(i, SWB_TENV_WIDTH)], &sy[i_mult(i, SWB_TENV_WIDTH)]);
    enerEnv = L_add(enerEnv, temp32);
    log2_tmp = L_mls(temp32,1638);

    Log2(log2_tmp, &log2_exp,  &log2_frac);
    log_rms_fix[NUM_PRE_SWB_TENV+i] = add(shl(log2_exp,10), shr(log2_frac,5)); move16();
  }
  ener_total_fix = L_add(ener_total_fix, enerEnv);
  enc_st->enerEnvPre[0] = enc_st->enerEnvPre[1]; move32();
  enc_st->enerEnvPre[1] = enerEnv; move32();

  log2_tmp = L_mls(ener_total_fix,137);  /* (NUM_FRAME * SWB_T_WIDTH)  240 */
  Log2(log2_tmp, &log2_exp,  &log2_frac);
  gain_fix = add(shl(log2_exp,10), shr(log2_frac,5));

  i_max_fix = 0; move16();
  max_deviation_fix = 0; move16();
  max_rise_fix = 0; move16();
  FOR (i = 0; i < (NUM_FRAME*SWB_TENV); i++)
  {
    if (sub(log_rms_fix[i], i_max_fix) > 0)
    {
      pos = i;  move16();
    }
    i_max_fix = s_max(i_max_fix , log_rms_fix[i]);  


    temp_fix = abs_s(sub(log_rms_fix[i], gain_fix));
    max_deviation_fix = s_max(max_deviation_fix, temp_fix);
  }

  FOR (i = 0; i < (NUM_FRAME*SWB_TENV-1); i++)
  {
    temp_fix = sub(log_rms_fix[i+1], log_rms_fix[i]);	
    max_rise_fix = s_max(max_rise_fix, temp_fix);
  }

  *transient = 0;  move16();
  test(); test();
  IF ( sub(max_deviation_fix, 6758) > 0 && sub(max_rise_fix, 4915) > 0 && sub(gain_fix, 16384) > 0) /* Q(11) */
  {
    *transient = 1;  move16();
  }

  test();
  IF (sub(*transient, 1) == 0 || sub(preMode, TRANSIENT) == 0)
  {
    IF (sub(pos, 4) >= 0)
    {
      temp_fix = shr(log_rms_fix[pos], 3);
      calc_avrg(pos, log_rms_fix, temp_fix, &avrg_fix);
      IF (sub(pos, 8) < 0)
      {
        calc_avrg(sub(11, pos), &log_rms_fix[pos+1], temp_fix, &avrg1_fix);
      }
    }
    FOR (i=0; i<SWB_TENV; i++)
    {
      srms[i] = s_min(30720, log_rms_fix[i+SWB_TENV]); move16();
    }
    IF (sub(*transient, 1) == 0)
    {
      i1 = s_min(sub(pos, SWB_TENV), SWB_TENV);
      IF(i1 > 0)
      {
        FOR (i=0; i<i1; i++)
        {			
          srms[i] = s_max(0, sub(srms[i], 2048)); move16();
        }
        test();
        IF((sub(i1, SWB_TENV) != 0)&&(s_and(avrg_fix,avrg1_fix) != 0))
        {
          srms[i1] = s_min(30720, add(srms[i1], 1024)); move16();
        }
      }
    }

    max_rms_fix = MaxArray(SWB_TENV, srms, &pos);

    ener_front_fix = L_mac0_Array(HALF_SUB_SWB_T_WIDTH, &enc_st->pre_sy[i_mult(SUB_SWB_T_WIDTH, pos)], &enc_st->pre_sy[i_mult(SUB_SWB_T_WIDTH, pos)]);
    ener_behind_fix = L_mac0_Array(HALF_SUB_SWB_T_WIDTH, &enc_st->pre_sy[add(i_mult(SUB_SWB_T_WIDTH, pos), HALF_SUB_SWB_T_WIDTH)], &enc_st->pre_sy[add(i_mult(SUB_SWB_T_WIDTH, pos), HALF_SUB_SWB_T_WIDTH)]);

    T_modify_flag = 0;   move16();
    if (L_sub(ener_behind_fix, ener_front_fix) > 0 )

    {
      T_modify_flag = 1;   move16();
    }
  }

  mov16(SWB_TENV, &enc_st->log_rms_fix_pre[SWB_TENV], enc_st->log_rms_fix_pre);
  mov16(SWB_TENV, &log_rms_fix[NUM_PRE_SWB_TENV], &enc_st->log_rms_fix_pre[SWB_TENV]);
  mov16(SWB_T_WIDTH, sy, enc_st->pre_sy);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return(T_modify_flag);
}


void Cod_fEnv(Word16 *sfEnv, Word16 *codword, Word16 mode)
{
  Word16 i, j, pos;
  Word16 temp;
  Word32 minn, L_temp;

  Word16 *pit = scodebookL;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Ptr);
    ssize += (UWord32) (2*SIZE_Word32); 
    ssize += (UWord32) ((3+1)*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  if(mode == 0)
  {
    pit = scodebookL;
  }
  if (sub(mode, 1) == 0)
  {
    pit = CodeBookH;
  }

  minn = 2147483647;  move32();
  pos = 0; move16();
  FOR (i=0; i<VQ_FENV_SIZE; i++)
  {
    L_temp = 0; move32();
    FOR (j=0; j<VQ_FENV_DIM; j++)
    {
      temp = sub(sfEnv[j], *pit);
      L_temp = L_mac0(L_temp, temp, temp);
      pit++;
    }

    if (L_sub(L_temp, minn) < 0)
    {
      pos = i; move16();
    }
    minn = L_min(minn, L_temp); 

  }

  temp = sub(VQ_FENV_SIZE, pos);
  temp = shl(temp, 2);

  pit -= temp;	
  mov16(VQ_FENV_DIM, pit, sfEnv);

  *codword = pos; move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

Word16 cod_fGain(Word32 *sfGain, Word16 sfSpectrumQ)
{
  Word16 index_fGain, frac;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (2*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  Log2(*sfGain, &index_fGain, &frac);
  if (sub(frac, 16384) > 0)
  {
    index_fGain = add(index_fGain, 1);
  }
  index_fGain = sub(index_fGain, sfSpectrumQ);
  index_fGain = bound(index_fGain, 0, 31);

  *sfGain = L_shl(0x1L, index_fGain);


  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return(index_fGain);
}

void calc_fEnv(Word16 index_fGain, 
               Word16 *sfSpectrum, 
               Word16 mode, 
               Word16 *sfEnv, 
               Word16 *index_codebook, 
               Word16 *index_fEnv,
               Word16 *sfEnv_unq,
               Word16 sfSpectrumQ
               )
{
  Word16 i,j;
  Word16 sgain_tmp;
  Word16 temp;
  Word32 L_temp;
  Word32 Sphere1, Sphere2;
  Word16 *spit;
  Word16 temp1, temp2;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Ptr);
    ssize += (UWord32) ((2+1+1+2)*SIZE_Word16);
    ssize += (UWord32) (3*SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  spit = sfSpectrum;		move16();	
  IF (sub(mode, TRANSIENT) == 0)
  {
    temp = shl(1, sub(15, index_fGain));
    temp1 = sub(4, sfSpectrumQ);
    temp2 = sub(16, sfSpectrumQ);

    L_temp = L_mult0(TRANSI_FENV_EXPAND, temp);  /* Q(15) */
    sgain_tmp = round_fx_L_shl(L_temp, 13);      /* Q(12) */

    FOR (i = 0; i < SWB_TRANSI_FENV; i++)
    {
      L_mac_shr(SWB_TRANSI_FENV_WIDTH, &L_temp, 2, spit);
      spit = spit + SWB_TRANSI_FENV_WIDTH;

      L_temp = L_mult0(L_sqrt(L_temp), sgain_tmp);  /* Q(12+sfSpectrumQ) */
      index_fEnv[i] = s_min(round_fx_L_shl(L_temp, temp1), 15); move16();

      j = shl(i, 1);
      sfEnv_unq[j]   = round_fx_L_shl( L_temp, temp2 );  move16(); /* Q(12) */
      sfEnv_unq[j+1] = sfEnv_unq[j];  move16();                    /* Q(12) */	
    }
  }
  ELSE
  {
    sgain_tmp = shl(1, sub(15, index_fGain));		
    calc_half_fenv(sfSpectrum, sgain_tmp, sfSpectrumQ, &Sphere1, sfEnv, sfEnv_unq);
    calc_half_fenv(&sfSpectrum[32], sgain_tmp, sfSpectrumQ, &Sphere2, &sfEnv[NORMAL_FENV_HALVE], &sfEnv_unq[NORMAL_FENV_HALVE]);
    if (L_sub(Sphere1, 56706990) > 0)
    {
      index_codebook[0] = 1; move16();
    }

    Cod_fEnv( sfEnv, index_fEnv, index_codebook[0] );

    if (L_sub(Sphere2, 56706990) > 0)
    {
      index_codebook[1] = 1; move16();
    }

    Cod_fEnv( &sfEnv[4], &index_fEnv[1], index_codebook[1] );
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

/* sharp classification */
void clas_sharp(Word16 preMod, Word16 *sSpectrum, Word32 sfGain, Word16 sQSpectrum, 
                Word16 *sharpMod, Word16 *noise_flag, Word16 spreGain
                , BWE_state_enc* enc_st    
                )
{
  Word16 i,j,k,noise;
  Word16 *wInput_Hi;
  Word16 sharp_fix[NUM_SHARP];
  Word16 sharpPeak_fix = 0;
  Word16  gain_tmp_fix;
  Word32  sharp_de;
  Word16 peak_fix;
  Word16 mag_fix;

  Word32 mean_32;
  Word16  mean_32_hi;
  Word16  mean_32_lo;

  move16();	

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Ptr);
    ssize += (UWord32) ((4 + NUM_SHARP + 6)*SIZE_Word16);
    ssize += (UWord32) (2*SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  wInput_Hi = sSpectrum;
  k=0;  move16();
  noise = 0;  move16();

  sharpPeak_fix = 16384;  move16();

  FOR (i = 0; i < NUM_SHARP; i ++)
  {
    peak_fix = 0; move16();
    mean_32 = 0; move32();

    FOR (j = 0; j < SHARP_WIDTH; j ++)
    {
      mag_fix = abs_s(*wInput_Hi);
      peak_fix = s_max(mag_fix, peak_fix);

      mean_32 = L_add(mean_32,L_deposit_l(mag_fix));
      wInput_Hi++;
    }
    L_Extract( mean_32, &mean_32_hi, &mean_32_lo );

    sharp_fix[i] = 0; move16();
    IF (mean_32 != 0 ) 
    {
      sharp_de =  L_sub(mean_32,L_deposit_l(peak_fix));
      sharp_fix[i] = div_l( L_shl(sharp_de,13),peak_fix);   move16(); /* Q(12) */
    }

    test();
    IF (sub(4096, mult(sharp_fix[i], 26214)) > 0 
      && sub(peak_fix, mult(shl(1, add(sQSpectrum, 4)), 20480)) > 0)
    {
      k = add(k, 1);
    }
    ELSE IF (sub(4096, mult(sharp_fix[i],16384)) < 0)
    {
      noise = add(noise, 1);
    }
    sharpPeak_fix = s_min(sharpPeak_fix, sharp_fix[i]);
  }

  j = 5;  move16();
  if(sub(preMod, HARMONIC) == 0)
  {
    j = 4;  move16();
  }
  if (sub(preMod, TRANSIENT) == 0)
  {
    j = 7;  move16();
  }
  gain_tmp_fix = extract_l(L_mls(sfGain, spreGain)); /* sQSpectrum */

  test(); test();
  IF(sub(k, j) >= 0 && sub(gain_tmp_fix, mult(shl(1,sQSpectrum), 16384)) > 0 
    && sub(gain_tmp_fix, mult(shl(1, add(sQSpectrum, 1)),29491)) < 0)
  {
    *sharpMod = 1;  move16();
    if (sub(enc_st->modeCount, 8) < 0)
    {
      enc_st->modeCount = add(enc_st->modeCount, 1);
    }
  }
  ELSE
  {
    *sharpMod = 0;  move16();
    if (enc_st->modeCount > 0)
    {
      enc_st->modeCount = sub(enc_st->modeCount, 1);
    }
  }
  if (sub(enc_st->modeCount, 2) >= 0)
  {
    *sharpMod = 1;  move16();
  }

  *noise_flag = 0;  move16();
  test();
  IF (sub(noise, 6) > 0 && sub(4096, mult(sharpPeak_fix,22938)) < 0)
  {
    *noise_flag = 1;  move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return;
}

void norm_spectrum_bwe( Word16* fSpectrum, Word32* fGain , Word16 nb_coef)
{
  Word32 L_temp;
  Word16* pit;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Ptr);
    ssize += (UWord32) (1*SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  pit = fSpectrum;	
  L_mac_shr(nb_coef, &L_temp, 3, pit); 
  pit = pit + nb_coef;


  *fGain = L_shr(L_Frac_sqrtQ31(L_temp), 16);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

void QMF_mirror( Word16 *s, Word16 l )
{
  Word16 i;

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Word16);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  FOR (i = 0; i < l; i += 2)
  {
    s[i] = sub( 0, s[i] );  move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
} /* end of QMF_mirror() */

/* main encoding function */
void* bwe_encode_const(void)
{
  BWE_state_enc  *enc_st=NULL;

  enc_st = (BWE_state_enc *)malloc( sizeof(BWE_state_enc) );
  if (enc_st == NULL) return NULL;


  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (1*SIZE_Ptr);
#ifdef MEM_STT
    ssize += (UWord32)(2*SIZE_Word16);
    ssize += (UWord32)((SWB_T_WIDTH*2 + 1 + NUM_PRE_SWB_TENV)*SIZE_Word16);
    ssize += (UWord32)(((NUM_FRAME - 1)*SWB_TENV + NUM_FRAME - 1)*SIZE_Word32);
#endif
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  bwe_encode_reset( (void *)enc_st );

  return (void *)enc_st;
}

void  bwe_encode_dest( void *work )
{
  BWE_state_enc  *enc_st=(BWE_state_enc *)work;

  if (enc_st != NULL)
  {
    free( enc_st );
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
}

Word16 bwe_encode_reset( void *work )
{
  BWE_state_enc  *enc_st=(BWE_state_enc *)work;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize = (UWord32) (1*SIZE_Ptr);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  if (enc_st != NULL)
  {
    zero16(sizeof(BWE_state_enc)/2, (Word16 *)enc_st);
  }
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif    
  return ENCODER_OK;
}

Word16 bwe_enc( Word16         sBufin[],           /* (i): Input super-higher-band signal */
               UWord16        **pBit,             /* (o): Output bitstream               */
               void           *work,              /* (i/o): Pointer to work space        */
               Word16         *stEnv,             /* (i): Q(11) */
               Word16         transi,
               Word16         *cod_Mode,
               Word16         *sfEnv,             /* (o): Q(sfSpectrumQ) */
               Word16         *sfSpectrum,        /* (o): Q(sfSpectrumQ) */
               Word16         *index_g,
               Word16         T_modify_flag,
               Word16         sfEnv_unq[],        /* (o): Q(12) */
               Word16         *sfSpectrumQ
               )
{
  BWE_state_enc *enc_st=(BWE_state_enc *)work;    
  Word16 sharpMod, mode, index_fGain;
  Word16 index_fEnv[SWB_TRANSI_FENV], index_fEnv_codebook[NUM_FENV_CODEBOOK], index_fEnv_codeword[NUM_FENV_VECT];
  Word16 noise_flag;
  Word16 norm;
  Word16 sY[L_FRAME_WB];   
  Word16 i;
  Word32 ssfSpectrum[SWB_F_WIDTH];

  Word32 i_max;
  Word32 L_temp;
  Word16 Shift, temp;
  Word32 sfGain;
  Word16 senn;

  Word16 temp1;

  move16();	
  zero16( NUM_FENV_CODEBOOK, index_fEnv_codebook);

  /*****************************/
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize;
    ssize =  (UWord32) (1*SIZE_Ptr);
    ssize += (UWord32) ((3+SWB_TRANSI_FENV+NUM_FENV_CODEBOOK+NUM_FENV_VECT+2+L_FRAME_WB+1+4)*SIZE_Word16);
    ssize +=  (UWord32) ((SWB_F_WIDTH+2+1)*SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /*****************************/ 

  FOR(i=0; i<4; i++)
  {
    temp = add(stEnv[i], 1024);
    stEnv[i] = shr(temp, 11);    move16();
  }

  /* MDCT */
  /* folding of spectral envelope. */
  QMF_mirror( sBufin, L_FRAME_WB );

  /* MDCT on 80 samples in the 8-16kHz band */
  bwe_mdct( enc_st->sIn, sBufin, sY, &norm );

  i_max = 0;  move32();
  FOR (i=0; i<SWB_F_WIDTH; i++) 
  {
    ssfSpectrum[i] = L_mult(sY[i], 18318);       move32();  /* Q(norm+10) */
    i_max = L_max(i_max, L_abs(ssfSpectrum[i]));
  }

  Shift = norm_l(i_max);
  FOR (i=0; i<SWB_F_WIDTH; i++) 
  {
    sfSpectrum[i] = round_fx_L_shl(ssfSpectrum[i], Shift);  /* Q(norm-6+Shift) */
    move16();
  }
  temp = add(norm, Shift);
  *sfSpectrumQ = sub(temp, 6); 

  norm_spectrum_bwe( sfSpectrum, &sfGain, SWB_F_WIDTH);
  senn = 0;        move16();
  IF(L_sub(sfGain, 32767) <= 0)
  {
    senn = 1;       move16();
    IF(sfGain !=0)
    {
      temp1 = extract_l(sfGain);
      temp = shl(1, *sfSpectrumQ);
      if (sub(temp1, temp) >= 0) 
      {
        senn = div_s(temp, temp1);  /* Q(15) */
      }
    }
    ELSE
    {
      *sfSpectrumQ = 0;
    }
  }

  test();
  IF (sub(transi, 1) == 0 || sub(enc_st->preMode, TRANSIENT) == 0) /* mode code: 11 */
  {	
    mode = TRANSIENT;       move16();
    *cod_Mode = mode; move16();
    PushBit((Word16)mode, pBit, 2);

    /* encode fGain with 5bits */
    index_fGain = cod_fGain( &sfGain, *sfSpectrumQ);
    PushBit((Word16)index_fGain, pBit, 5 );
    calc_fEnv( index_fGain, sfSpectrum, mode, sfEnv, index_fEnv_codebook, index_fEnv, 
      sfEnv_unq, *sfSpectrumQ);
    temp = round_fx_L_shl(6554, add(*sfSpectrumQ, index_fGain));
    FOR (i = 0; i < SWB_TRANSI_FENV; i++)
    {
      PushBit(index_fEnv[i], pBit, 4);
      sfEnv[i] = round_fx_L_shl_L_mult(temp, index_fEnv[i], 16); move16();
    }

    FOR (i=0; i<SWB_TENV; i++)
    {
      PushBit(stEnv[i], pBit, 4);
    }

    PushBit((Word16)T_modify_flag, pBit, 1 );

    if(sub(transi, 1) != 0)
    {
      mode = NORMAL; move16();
    }

    enc_st->modeCount = 0;  move16();
    temp = round_fx_L_shl(6554, add(*sfSpectrumQ, index_fGain));
  }
  ELSE /* not transient */
  {
    /* classification	*/
    clas_sharp(enc_st->preMode, sfSpectrum,  sfGain, *sfSpectrumQ,
      &sharpMod, &noise_flag, enc_st->preGain
      , enc_st
      );

    /* encode fGain with 5bits */
    index_fGain = cod_fGain( &sfGain, *sfSpectrumQ);

    IF (sub(sharpMod, 1) == 0 || sub(enc_st->preMode, HARMONIC) == 0)
    {
      mode = HARMONIC;  move16();
      *cod_Mode = mode; move16();
      PushBit(mode, pBit, 2 );
      if (sub(sharpMod, 1) != 0)
      {
        mode = NORMAL; move16();
      }
    }
    ELSE
    {
      mode = NORMAL; move16();
      *cod_Mode = mode; move16();
      PushBit( mode, pBit, 1 );
      PushBit(noise_flag, pBit, 1 );
    }
    calc_fEnv( index_fGain, sfSpectrum, mode, sfEnv, index_fEnv_codebook, index_fEnv_codeword   
      , sfEnv_unq, *sfSpectrumQ );

    PushBit( index_fGain, pBit, 5 );

    PushBit( index_fEnv_codebook[0], pBit, 1 );
    PushBit( index_fEnv_codebook[1], pBit, 1 );

    PushBit( index_fEnv_codeword[0], pBit, 6 );
    PushBit( index_fEnv_codeword[1], pBit, 6 );

    temp1 = add(*sfSpectrumQ, 4);

    FOR (i=0; i<SWB_NORMAL_FENV; i++)
    {
      L_temp = L_shl(sfEnv[i], index_fGain);
      sfEnv[i] = round_fx_L_shl(L_temp, temp1);  /* Q(sfSpectrumQ) */
      move16();
    }	
  }

  enc_st->preMode = mode;       move16();
  enc_st->preGain = senn;       move16();

  *index_g = index_fGain;       move16();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/

  return ENCODER_OK;
}

