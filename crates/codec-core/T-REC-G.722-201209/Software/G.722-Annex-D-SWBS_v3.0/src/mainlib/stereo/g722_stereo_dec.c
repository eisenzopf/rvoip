/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

#ifdef LAYER_STEREO

#include <math.h>
#include "g722_stereo.h"
#include "fft.h"
#include "qmfilt.h"
#include "stereo_tools.h"
#include "pcmswb_common.h"
#include "bwe_mdct.h"
#include "bwe.h"
#include "stdio.h"
#include "math_op.h"
#include "rom.h"

#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif

#ifdef WMOPS
extern short Id;
extern short Id_dmx;
extern short Id_fft;
extern short Id_ifft;
extern short Id_st_enc;
extern short Id_itd;
extern short Id_st_dec;
#endif

static Word16 calcIPD(Word16 mode, Word16 stereo_mono_flag, Word16 fb_ipd, 
                      Word16 fb_itd, Word16 *c, Word16 *mono_dec_real, 
                      Word16 *mono_dec_imag, Word16 *L_mag, Word16 *R_mag, 
                      Word16 *L_real_syn, Word16 *L_imag_syn, 
                      Word16 *R_real_syn, Word16 *R_imag_syn);
static void postProcStereo(Word16 n, Word16 *gainPost, Word16 *monoReal, Word16 *monoImag); 
static void boundPostProc(Word16 boundPost, Word16 *monoReal, Word16 *monoImag); 
static void dequantILD(Word16 frame_idx, Word16 *mem_ILD_q, Word16 *r1ws_pt);
static Word16 dequantRefineILD(Word16 swb_flag, Word16 ic_flag, Word16 frame_idx, Word16 idx, 
                               Word16 *mem_ILD_q, Word16 *r1ws_pt);
static void dequantILD0(Word16 nb4, Word16 nb3, Word16 *mem_ild_q, Word16 *r1ws_pt);
static void calc_ICSyntScaleFactors(Word16 n, Word16 *pre_ild_q, Word16 icSq, 
                                    Word16 *w1_d_c1, Word16 *w2_d_c2, Word16 *w3);
static Word16 calcRelPowerLR(Word16 icSq, Word16 pre_ild_q);
static Word16 calcScaleFacCorrel(Word16 val1, Word32 Lval2, Word16 val3);
static Word16 calcScaleFacUncorrel(Word16 val1, Word16 val3);
static void decorrel(Word16 n, Word16 region_ic, Word16 *w3, 
                     Word16 *w1_d_c1, Word16 *w2_d_c2,
                     Word16 *memDecorrReal, Word16 *memDecorrImag, 
                     Word16 *L_real_syn, Word16 *L_imag_syn, Word16 q_left,
                     Word16 *R_real_syn, Word16 *R_imag_syn, Word16 q_right);
static Word16 decorrelOnePoint(Word16 mem, Word16 val_io, Word16 val3, Word16 val1, Word16 qVal);
static Word16 decod_smoothIC(Word16 ic_flag, Word16 ic_idx, Word16 *ic_sm);
static Word16 stereoSynth_ITD_IPD(Word16 mode, Word16 SWB_WB_flag, 
                                  Word16 stereo_mono_flag, Word16 *bpt_stereo,
                                  Word16 fb_ipd, Word16 fb_itd, Word16 *pre_ild_q, 
                                  Word16 *pre_ipd_q, Word16 swb_ILD_mode,  
                                  Word16 *swb_frame_idx, Word16 *c, 
                                  Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                                  Word16 *L_mag, Word16 *R_mag, Word16 *L_real_syn, 
                                  Word16 *L_imag_syn, Word16 *R_real_syn, Word16 *R_imag_syn);
static void calc_Phase_syn_ITD(Word16 nb, Word16 iAngleStep, 
                               Word16 iAngleRotate, Word16 *mono_dec_real, 
                               Word16 *mono_dec_imag, Word16 *c, 
                               const Word16 *ptr_cidx, Word16 *L_mag, Word16 *R_mag, 
                               Word16 *L_real_syn, Word16 *L_imag_syn, 
                               Word16 *R_real_syn, Word16 *R_imag_syn);
static void calc_Phase_syn_ITD0(Word16 nb, Word16 *mono_dec_real, 
                                Word16 *mono_dec_imag, Word16 *c, 
                                const Word16 *ptr_cidx, Word16 *L_mag, Word16 *R_mag, 
                                Word16 *L_real_syn, Word16 *L_imag_syn, 
                                Word16 *R_real_syn, Word16 *R_imag_syn);
static void dequantIPD(Word16 mode, Word16 SWB_WB_flag, Word16 *bpt_stereo, 
                       Word16 fb_ipd, Word16 *pre_ipd_q, 
                       Word16 stereo_mono_flag, Word16 swb_ILD_mode,  
                       Word16 *swb_frame_idx,Word16 *mono_dec_real, 
                       Word16 *mono_dec_imag, Word16 *c, Word16 *L_mag,
                       Word16 *R_mag, Word16 *L_real_syn, Word16 *L_imag_syn, 
                       Word16 *R_real_syn, Word16 *R_imag_syn);
static void dequantIPD_5bits(Word16 nb, Word16 *bpt_stereo, Word16 fb_ipd, 
                             Word16 *pre_ipd_q, Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                             const Word16 *c_idx, Word16 *c, Word16 *L_mag, Word16 *R_mag, 
                             Word16 *L_real_syn, Word16 *L_imag_syn, 
                             Word16 *R_real_syn, Word16 *R_imag_syn);
static void dequantOneIPD_5bits(Word16 *bpt_stereo, Word16 fb_ipd, 
                                Word16 *pre_ipd_q, 
                                Word16 mono_dec_real, Word16 mono_dec_imag, 
                                Word16 c, Word16 L_mag, Word16 R_mag, 
                                Word16 *L_real_syn, Word16 *L_imag_syn, 
                                Word16 *R_real_syn, Word16 *R_imag_syn);
/* prototype for stereo_mono_flag = 0 */
static void dequantIPD_5bits_0(Word16 nb, Word16 *bpt_stereo, Word16 fb_ipd, 
                               Word16 *pre_ipd_q, Word16 *mono_dec_real, 
                               Word16 *mono_dec_imag, const Word16 *c_idx, 
                               Word16 *c, Word16 *L_mag, Word16 *R_mag, 
                               Word16 *L_real_syn, Word16 *L_imag_syn, 
                               Word16 *R_real_syn, Word16 *R_imag_syn);
static void dequantOneIPD_5bits_0(Word16 *bpt_stereo, Word16 fb_ipd, 
                                  Word16 *pre_ipd_q, Word16 mono_dec_real, Word16 mono_dec_imag, 
                                  Word16 c, Word16 L_mag, Word16 R_mag, 
                                  Word16 *L_real_syn, Word16 *L_imag_syn, 
                                  Word16 *R_real_syn, Word16 *R_imag_syn);
void Phase_syn_IPD0(Word16 IPD_q,Word16 mono_dec_real,Word16 mono_dec_imag,
                    Word16 c,Word16 L_mag,Word16 R_mag,Word16 *L_real_syn,
                    Word16 *L_imag_syn,Word16 *R_real_syn,Word16 *R_imag_syn);
void Phase_syn_ITD0(Word16 mono_dec_real, Word16 mono_dec_imag,Word16 c,
                    Word16 L_mag,Word16 R_mag,Word16 *L_real_syn,Word16 *L_imag_syn,
                    Word16 *R_real_syn,Word16 *R_imag_syn);

/*************************************************************************
* g722_stereo_decode_const
*
* g722 stereo decoder constructor
**************************************************************************/
void *g722_stereo_decode_const()
{
    g722_stereo_decode_WORK *w = NULL;

    w = (g722_stereo_decode_WORK *)malloc(sizeof(g722_stereo_decode_WORK));
    if (w != NULL) {
        g722_stereo_decode_reset((void *)w);
    }
    return (void *)w;
}

/*************************************************************************
* g722_stereo_decode_dest
*
* g722 stereo decoder destructor
**************************************************************************/
void g722_stereo_decode_dest( void* ptr )
{
    g722_stereo_decode_WORK *w = (g722_stereo_decode_WORK *)ptr;
    if (w != NULL) {
        free(w);
    }
    return;
}

/*************************************************************************
* g722_stereo_decode_reset
*
* g722 stereo decoder reset
**************************************************************************/
void g722_stereo_decode_reset( void* ptr )
{
    g722_stereo_decode_WORK *w = (g722_stereo_decode_WORK *)ptr;
    IF (w != NULL)
    {
        w->c1_swb[0]        = 16384;move16();//Q14
        w->c2_swb[0]        = 16384;move16();//Q14
        w->c1_swb[1]        = 16384;move16();//Q14
        w->c2_swb[1]        = 16384;move16();//Q14
        w->swb_ILD_mode     = 0;move16();
        w->pre_swb_ILD_mode = 0;move16();
        w->delay            = 0;move16();
        w->swb_frame_idx    = 0;move16();
        w->c_flag           = 0;move16();
        w->pre_flag         = 0;move16();
        w->pre_fb_itd       = 0;move16();
        w->fb_ipd           = 0;move16();
        w->fb_itd           = 0;move16();
        w->pre_fb_ipd       = 0;move16();
        w->pre_norm_left    = 0;move16();
        w->pre_norm_right   = 0;move16();


        zero16(20,w->mem_ILD_q);
        zero16(20,w->pre_ild_q);
        zero16(25,w->pre_ipd_q);

        zero16(58, w->mem_output_L);
        zero16(58, w->mem_output_R);
        zero16(58, w->mem_mono_win);
        zero16(L_FRAME_WB, w->sCurSave_left);
        zero16(L_FRAME_WB, w->sCurSave_right);
        zero16(L_FRAME_WB, w->mem_mono);
        zero16(L_FRAME_WB, w->mem_left_mdct);
        zero16(L_FRAME_WB, w->mem_right_mdct);
        zero16(G722_SWBST_D_COMPENSATE,w->mem_left);
        zero16(G722_SWBST_D_COMPENSATE,w->mem_right);

        w->pre_ic_flag  = 0;move16();
        w->pre_ic_idx   = 0;move16();

        zero16((NFFT/2 + 1) * 4,w->mem_decorr_real);
        zero16((NFFT/2 + 1) * 4,w->mem_decorr_imag);
        w->ic_sm = 0; move16();
        zero16(4,w->q_mem);
        zero16(8,w->log_rms_pre);
        zero32(2,w->enerEnvPre);
        w->mode     = 0; move16();
        w->pre_mode = 0; move16();
        const16(WB_POSTPROCESS_WIDTH, 16384, w->spGain_sm_wb);
        zero16(L_FRAME_WB, w->mdct_mem);
    }
    return;
}

/*************************************************************************
* Trans_detect_dec
*
* wideband transient detection for post processing
**************************************************************************/
Word16 Trans_detect_dec(Word16 *sy,         /* (i)  decoded g.722 signal  */ /* Q(0)  */
                        void* work    
                        )
{
    Word16  i, pos;
    Word16 transient;
    Word16 log_rms[NUM_FRAME * SWB_TENV]; /* Q(11) */    
    Word16 temp, i_max, max_deviation; /* Q(11) */
    Word16 max_rise;
    Word32 ener_total;
    Word32 log2_tmp;
    Word16 log2_exp;
    Word16 log2_frac;
    Word32 temp32;
    Word16 gain;
    Word16 *pit;

    Word32 enerEnv;
    g722_stereo_decode_WORK *enc_st = (g722_stereo_decode_WORK *)work;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((10 + 1 * (NUM_FRAME * SWB_TENV)) * SIZE_Word16 +  3 * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif

    pos = 0; move16();
    enerEnv = 0; move32();
    ener_total = 0; move32();

    pit = sy;

    mov16(NUM_PRE_SWB_TENV, enc_st->log_rms_pre, log_rms);

    ener_total = L_add(enc_st->enerEnvPre[0], enc_st->enerEnvPre[1]);

    FOR (i = 0; i < SWB_TENV; i++)  /* 0 --- 4 */
    {
        temp32 = L_mac0_Array(SWB_TENV_WIDTH, &sy[i_mult(i, SWB_TENV_WIDTH)], &sy[i_mult(i, SWB_TENV_WIDTH)]);
        enerEnv = L_add(enerEnv, temp32);
        log2_tmp = L_mls(temp32,1638);
        Log2(log2_tmp, &log2_exp,  &log2_frac);
        log_rms[NUM_PRE_SWB_TENV+i] = add(shl(log2_exp,10), shr(log2_frac,5)); move16();
    }
    ener_total = L_add(ener_total, enerEnv);
    enc_st->enerEnvPre[0] = enc_st->enerEnvPre[1]; move32();
    enc_st->enerEnvPre[1] = enerEnv; move32();

    log2_tmp = L_mls(ener_total,137);  /* (NUM_FRAME * SWB_T_WIDTH)  240 */
    Log2(log2_tmp, &log2_exp,  &log2_frac);
    gain = add(shl(log2_exp,10), shr(log2_frac,5));

    i_max = 0; move16();
    max_deviation = 0; move16();
    max_rise = 0; move16();
    FOR (i = 0; i < (NUM_FRAME*SWB_TENV); i++)
    {
        if (sub(log_rms[i], i_max) > 0)
        {
            pos = i;  move16();
        }
        i_max = s_max(i_max , log_rms[i]);  
        temp = abs_s(sub(log_rms[i], gain));
        max_deviation = s_max(max_deviation, temp);
    }

    FOR (i = 0; i < (NUM_FRAME*SWB_TENV-1); i++)
    {
        temp = sub(log_rms[i+1], log_rms[i]);   
        max_rise = s_max(max_rise, temp);
    }

    transient = 0;  move16();
    test(); test();
    IF ( sub(max_deviation, 6144) > 0 && sub(max_rise, 4813) > 0 && sub(gain, 16384) > 0) /* Q(11) */
    {
        transient = 1;  move16();
    }
    test();
    mov16(SWB_TENV, &enc_st->log_rms_pre[SWB_TENV], enc_st->log_rms_pre);
    mov16(SWB_TENV, &log_rms[NUM_PRE_SWB_TENV], &enc_st->log_rms_pre[SWB_TENV]);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(transient);
}

/*************************************************************************
* g722_stereo_decode
*
* G.722 stereo decoder
**************************************************************************/
void g722_stereo_decode(Word16* bpt_stereo,
                        Word16* mono_dec,
                        Word16* L_syn,
                        Word16* R_syn,
                        void*   ptr,
                        Word16  mode,
                        Word16  SWB_WB_flag,
                        Word16  ploss_status,
                        Word16* spGain_sm,
                        Word16  cod_Mode,
                        Word16  stereo_mono_flag
                        )
{
    g722_stereo_decode_WORK *w = (g722_stereo_decode_WORK *)ptr;        
    Word16 i, j;
    Word16 flag;
    Word16 *r1ws_pt = bpt_stereo;
    Word16 frame_idx;
    Word16 idx0, idx1;
    Word16 region_ic;
    Word16 mono_dec_real[NFFT + 2];
    Word16 *mono_dec_imag = &mono_dec_real[NFFT/2 + 1]; 
    Word16 q_mono;
    Word16 L_real_syn[NFFT + 2];           
    Word16 *L_imag_syn = &L_real_syn[NFFT/2 + 1];
    Word16 R_real_syn[NFFT + 2];            
    Word16 *R_imag_syn = &R_real_syn[NFFT/2 + 1];
    Word16 q_left, q_right;
    Word16 c[20];
    Word16 L_mag[81],R_mag[81];
    Word16 ic_flag = 0;
    Word16 ic_idx = 0;
    Word16 tmp16;
    Word16 w1_d_c1[IC_END_BAND - IC_BEGIN_BAND];
    Word16 w2_d_c2[IC_END_BAND - IC_BEGIN_BAND];
    Word16 w3[IC_END_BAND - IC_BEGIN_BAND];

    Word16 mono_mdct[L_FRAME_WB],norm_mono;
    Word16 mono_mdct1[L_FRAME_WB],norm_MDCT;
    Word16 *spMDCT_wb;
    Word16 spGain[WB_POSTPROCESS_WIDTH];

    Word16 nbBit;
    move16(); move16(); move16();

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((14 + 3 * (NFFT + 2) + 20 + 2 * 81 + 3 * (IC_END_BAND - IC_BEGIN_BAND) + 
        2 * (L_FRAME_WB) + WB_POSTPROCESS_WIDTH) * SIZE_Word16 
        +  0 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif

#ifdef WMOPS_IDX
    setCounter(Id_st_dec);
#endif

    /* windowing + FFT decoded mono */
    windowStereo(mono_dec, w->mem_mono_win, mono_dec_real);
    fixDoRFFTx(mono_dec_real,&q_mono);

    IF(SWB_WB_flag == 0)
    { /* WB case */ 
        /* zeroing the interval [7000, 8000 Hz] */
        FOR(j=71; j<(NFFT/2+1); j++)
        {
            mono_dec_real[j] = 0;move16();
            mono_dec_imag[j] = 0;move16();
        }
        /*post processing is only performed for the normal frame*/
        IF(ploss_status == 0)
        {
            w->mode = Trans_detect_dec(mono_dec,w);
            IF(w->mode == 0)
            {
                /* MDCT on 80 samples in the 0-8kHz band */
                bwe_mdct( w->mdct_mem, mono_dec, mono_mdct, &norm_mono );
                array_oper(L_FRAME_WB, 18318, mono_mdct, mono_mdct1, &mult);
                norm_MDCT = sub(norm_mono, 6);

                /* postprocess 4000-8000 Hz */
                spMDCT_wb = &mono_mdct1[L_FRAME_WB - WB_POSTPROCESS_WIDTH];

                MaskingFreqPostprocess(spMDCT_wb, WB_POSTPROCESS_WIDTH, spGain,22938, norm_MDCT);
                IF (sub(w->pre_mode, 1) == 0)
                {
                    mov16(WB_POSTPROCESS_WIDTH, spGain, w->spGain_sm_wb);
                } /* end w->pre_mode =  1*/
                ELSE
                {
                    FOR (i = 0; i < WB_POSTPROCESS_WIDTH; i++) 
                    {
                        w->spGain_sm_wb[i] = extract_l_L_shr(L_mac0(L_deposit_l(w->spGain_sm_wb[i]), 
                                                                    spGain[i], 1), 1); /* Q(14) */ 
                        move16();
                    }
                }  /* end w->pre_mode !=  1*/
                postProcStereo(WB_POSTPROCESS_WIDTH, w->spGain_sm_wb, &mono_dec_real[44], &mono_dec_imag[44]); 
            }
            ELSE
            { /* w->mode = 0 */
                const16(WB_POSTPROCESS_WIDTH, 16384, w->spGain_sm_wb);
                mov16(L_FRAME_WB,mono_dec,w->mdct_mem);
            } /* end w->mode = 0 */
            w->pre_mode = w->mode; move16();
        } /* end ploss_status = 0 */
    } /* end WB case */ 
    ELSE
    { /* SWB case */ 
        IF(ploss_status == 0)
        {
            postProcStereo(WB_POSTPROCESS_WIDTH, spGain_sm, &mono_dec_real[44], &mono_dec_imag[44]);
        }
    } /* end SWB case */ 

    IF(ploss_status == 0)       
    {
        /*read the frame mode indication*/
        w->fb_ipd      = w->pre_fb_ipd;     move16();
        w->fb_itd      = w->pre_fb_itd;     move16();
        ic_flag        = w->pre_ic_flag;    move16();

        read_index1(r1ws_pt,&flag);
        r1ws_pt += 1;
        IF (flag == 0) /*4 frames mode*/
        {
            w->c_flag = 1;move16();
            read_index2( r1ws_pt, &frame_idx);
            r1ws_pt += 2;  
            /*decoded ILD*/
            dequantILD(frame_idx, w->mem_ILD_q, r1ws_pt);
            r1ws_pt += 22;
            /* decod whether whole IPD or ITD in frame */
            read_index1(r1ws_pt, &idx0);
            r1ws_pt += 1;

            /*decode selected inter-channel difference*/
            read_index4(r1ws_pt, &idx1);
            r1ws_pt += 4;

            w->pre_fb_itd = 0; move16();
            IF( idx0 )
            {
                w->pre_fb_ipd  = 0;                        move16();
                /* assume whole wideband ITD is selected */
                w->pre_ic_flag = 0;                        move16();
                w->pre_fb_itd  = sub(idx1, 7);             move16();
                IF(sub(idx1,15) == 0) /* whole wideband IC is selected */
                {
                    ic_idx        = w->pre_ic_idx;         move16();
                    read_index2( r1ws_pt, &w->pre_ic_idx);
                    r1ws_pt += 2;
                    w->pre_ic_flag = 1;                    move16();
                    w->pre_fb_itd  = 0;                    move16();
                }
            }
            ELSE /* whole wideband IPD is selected */
            {
                w->pre_fb_ipd  = tab_phase_q4[idx1];       move16();
                w->pre_ic_flag = 0;                        move16();
            }
            w->delay = w->fb_itd;                          move16();
            read_index2(r1ws_pt, &idx1);
            r1ws_pt += 2;
            nbBit = dequantRefineILD(SWB_WB_flag, w->pre_ic_flag, frame_idx, 
                                     idx1, w->mem_ILD_q, r1ws_pt);
            r1ws_pt  += nbBit;
        }
        ELSE /*decode ILD in 2 frames mode*/
        {
            read_index1( r1ws_pt, &frame_idx);
            r1ws_pt += 1;  
            dequantILD0(sub(5, SWB_WB_flag), add(4,SWB_WB_flag), &w->mem_ILD_q[frame_idx], r1ws_pt);
            r1ws_pt += sub(37, SWB_WB_flag);
            /* Switch from 4 frames mode to 2 frames mode => change all bands */
            IF(w->c_flag)
            {
                IF(frame_idx == 0)
                {
                    mov16_ext(10, w->mem_ILD_q, 2, &w->mem_ILD_q[1], 2);
                    w->c_flag = 0;                   move16();
                }
            }

            /* reset full band itd and full band IPD when 2 frames mode (quick change of ILD) */
            w->pre_fb_itd  = 0;                      move16();
            w->pre_fb_ipd  = 0;                      move16();
            w->pre_ic_flag = 0;                      move16();
        }

        IF(SWB_WB_flag)
        {
            read_index1( r1ws_pt, &w->swb_ILD_mode);
            r1ws_pt += 1;
        }
    }

    bpt_stereo += 39;
#ifdef WMOPS_IDX
    setCounter(Id_itd);
#endif
    /*recover the amplitudes of left and right channels*/
    stereo_synthesis(w->pre_ild_q, mono_dec_real, mono_dec_imag, q_mono, 
                     L_real_syn, L_imag_syn, R_real_syn, R_imag_syn,
                     &q_left, &q_right, L_mag, R_mag, ploss_status);

#ifdef WMOPS_IDX
    setCounter(Id_st_dec);
#endif

    /* IPD synthesis */
    IF(ploss_status == 0)
    {
        region_ic = stereoSynth_ITD_IPD(mode, SWB_WB_flag, stereo_mono_flag, bpt_stereo,
                                        w->fb_ipd,w->fb_itd, w->pre_ild_q, 
                                        w->pre_ipd_q, w->swb_ILD_mode, &w->swb_frame_idx, 
                                        c, mono_dec_real, mono_dec_imag, L_mag, R_mag, 
                                        L_real_syn, L_imag_syn, R_real_syn, R_imag_syn);
    }
    /*IC synthesis*/
    IF(ploss_status == 0)
    {
        tmp16 = decod_smoothIC(ic_flag, ic_idx, &w->ic_sm);
        /*Calculate the relative power of left and right channels*/
        i = c_idx[region_ic]; move16();
        j = sub(IC_END_BAND,i);
        /*calculate the scale factor*/
        calc_ICSyntScaleFactors(j, &w->pre_ild_q[i], sub(32767, tmp16),
                                &w1_d_c1[i],&w2_d_c2[i], &w3[i]);
        decorrel(sub(NFFT/2 + 1, region_ic), region_ic,w3, w1_d_c1,
                 w2_d_c2, w->mem_decorr_real, w->mem_decorr_imag, 
                 L_real_syn, L_imag_syn, sub(q_left, w->q_mem[0]),
                 R_real_syn, R_imag_syn, sub(q_right, w->q_mem[2]));
        FOR(i = 0; i< 20; i++)
        {
            w->pre_ild_q[i] = shr(w->mem_ILD_q[i],9); move16();
        }
        /*update the memory*/
        FOR(i=0;i<(NFFT/2 + 1) * 3;i++)
        {
            w->mem_decorr_real[i] = w->mem_decorr_real[NFFT/2 + 1 + i]; move16();
            w->mem_decorr_imag[i] = w->mem_decorr_imag[NFFT/2 + 1 + i]; move16();
        }

        /*different delays are applied to the decoded mono signal to generate the de-correlated signals*/
        IF((w->mode == 0 && SWB_WB_flag == 0) ||(sub(cod_Mode,TRANSIENT) != 0 && SWB_WB_flag == 1))
        {
            FOR(i=0;i<NFFT/2 + 1;i++)
            {
                w->mem_decorr_real[(NFFT/2 + 1) * 3 + i] = mono_dec_real[i]; move16();
                w->mem_decorr_imag[(NFFT/2 + 1) * 3 + i] = mono_dec_imag[i]; move16();
            }
        }
        ELSE /*if the mono signal is classified as transient signal, update the memory by zeros*/
        {
            zero16(NFFT/2,&w->mem_decorr_real[(NFFT/2 + 1) * 3]);
            zero16(NFFT/2,&w->mem_decorr_imag[(NFFT/2 + 1) * 3]);
        }
        w->q_mem[0] = w->q_mem[1]; move16();
        w->q_mem[1] = w->q_mem[2]; move16();
        w->q_mem[2] = w->q_mem[3]; move16();
        w->q_mem[3] = q_mono; move16();
    }
    ELSE
    {
        zero16(NFFT/2,&w->mem_decorr_real[(NFFT/2 + 1) * 3]);
        zero16(NFFT/2,&w->mem_decorr_imag[(NFFT/2 + 1) * 3]);
    }

    /*IFFT decoded left and right channel signal*/
    q_left  = sub(q_left, 16);
    q_right = sub(q_right, 16);

    fixDoRiFFTx(L_real_syn,&q_left);
    fixDoRiFFTx(R_real_syn,&q_right);
    /* Windowing */
    /* overlap and add  left and right channels */
    OLA( &L_real_syn[11], w->mem_output_L, L_syn);
    OLA( &R_real_syn[11], w->mem_output_R, R_syn);

    /* update memory */
    FOR(i=0; i< 58; i++)
    {
        w->mem_mono_win[i] = mono_dec[i + 22]; move16();
        w->mem_output_L[i] = mult(L_real_syn[i + NFFT/2 + 11], win_D[58 - i - 1]); move16();
        w->mem_output_R[i] = mult(R_real_syn[i + NFFT/2 + 11], win_D[58 - i - 1]); move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* decod_smoothIC
*
* inverse quantization of IC and smoothing
**************************************************************************/
static Word16 decod_smoothIC(Word16 ic_flag, Word16 ic_idx, Word16 *ic_sm)
{
    Word16 icSq, icLoc;
    Word16 tmp16, tmp16_2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((4) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    /*decode full band IC*/
    icLoc = 32767; move16(); /* 1.0 */
    IF(ic_flag)
    {
        icLoc = ic_table[ic_idx]; move16();
        tmp16 = mult_r(*ic_sm, 32244); /* 0.984 */
        tmp16_2 = mult_r(icLoc, 524); /* 0.016 */
        icLoc = add(tmp16, tmp16_2); 
    }
    *ic_sm = icLoc; move16();
    icSq = mult_r(icLoc, icLoc);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(icSq);
}

/*************************************************************************
* stereoSynth_ITD_IPD
*
* phase computation from ITD and IPD of frequency bins left and right channels
**************************************************************************/
static Word16 stereoSynth_ITD_IPD(Word16 mode, Word16 SWB_WB_flag, 
                                  Word16 stereo_mono_flag, Word16 *bpt_stereo,
                                  Word16 fb_ipd, Word16 fb_itd, Word16 *pre_ild_q,
                                  Word16 *pre_ipd_q, Word16 swb_ILD_mode,  
                                  Word16 *swb_frame_idx, Word16 *c, 
                                  Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                                  Word16 *L_mag, Word16 *R_mag, Word16 *L_real_syn, 
                                  Word16 *L_imag_syn, Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 i, j, region_ic;
    Word16 iAngleStep, iAngleRotate;
    const Word16 *ptr_cidx; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((5) * SIZE_Word16 +  (0) * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif

    FOR (i = 2; i < 17; i++)
    {
        j = pre_ild_q[i];move16();
        c[i] = c_table10[80 + j]; move16();
    }

    iAngleStep = shr( extract_l(L_mult0( fb_itd ,CPIDNFFT_FQ15 )), 2);
    IF(sub(mode, MODE_R1ws) ==0) 
    {   /* case R1ws*/
        region_ic = IPD_SYN_START; move16();
        ptr_cidx = c_idx + region_ic; 
        IF(stereo_mono_flag == 0)
        {
            calc_Phase_syn_ITD0(42, &mono_dec_real[region_ic], &mono_dec_imag[region_ic], 
                                c, ptr_cidx, &L_mag[region_ic], &R_mag[region_ic],
                                &L_real_syn[region_ic], &L_imag_syn[region_ic], 
                                &R_real_syn[region_ic], &R_imag_syn[region_ic]);
        }
        ELSE 
        {
            iAngleRotate = extract_l(L_mult0( iAngleStep , region_ic));
            iAngleRotate = sub( fb_ipd ,iAngleRotate ); // Q12
            calc_Phase_syn_ITD(42, iAngleStep,iAngleRotate, &mono_dec_real[region_ic], 
                               &mono_dec_imag[region_ic], c, ptr_cidx, &L_mag[region_ic], 
                               &R_mag[region_ic], &L_real_syn[region_ic], &L_imag_syn[region_ic], 
                               &R_real_syn[region_ic], &R_imag_syn[region_ic]);
        }
    }
    ELSE 
    {  /* case mode != R1ws*/
        region_ic = sub(IPD_SYN_WB_END_WB+1, SWB_WB_flag); 
        if( sub(mode, MODE_R5ss) == 0)
        {
            region_ic = IPD_SYN_SWB_END_SWB + 1;move16();
        }

        j = sub(bands[17], region_ic); /* number of bins */
        ptr_cidx = c_idx + region_ic; 
        IF(stereo_mono_flag == 0)
        {
            calc_Phase_syn_ITD0(j, &mono_dec_real[region_ic], &mono_dec_imag[region_ic], 
                                c, ptr_cidx, &L_mag[region_ic], &R_mag[region_ic],
                                &L_real_syn[region_ic], &L_imag_syn[region_ic], 
                                &R_real_syn[region_ic], &R_imag_syn[region_ic]);
        }
        ELSE 
        {
            iAngleRotate = extract_l(L_mult0(fb_itd, region_ic));
            WHILE(sub(iAngleRotate,NFFT) >= 0) 
            {
                iAngleRotate = sub( iAngleRotate, NFFT );
            }

            WHILE(add(iAngleRotate,NFFT) <= 0) 
            {
                iAngleRotate = add( iAngleRotate, NFFT );
            }
            iAngleRotate = extract_l(L_mult0(iAngleRotate ,CPIDNFFT_FQ15_2)); //  Q12  -pi ~ pi
            iAngleRotate = sub( fb_ipd ,iAngleRotate ); // Q12
            calc_Phase_syn_ITD(j, iAngleStep, iAngleRotate, &mono_dec_real[region_ic], 
                               &mono_dec_imag[region_ic], c, ptr_cidx, &L_mag[region_ic], &R_mag[region_ic],
                               &L_real_syn[region_ic], &L_imag_syn[region_ic], 
                               &R_real_syn[region_ic], &R_imag_syn[region_ic]);
        }
        dequantIPD(mode, SWB_WB_flag, bpt_stereo, fb_ipd, pre_ipd_q, 
                   stereo_mono_flag, swb_ILD_mode,  swb_frame_idx,            
                   mono_dec_real, mono_dec_imag, c, L_mag, R_mag, 
                   L_real_syn, L_imag_syn, R_real_syn, R_imag_syn);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(region_ic);
}

/*************************************************************************
* calc_Phase_syn_ITD
*
* phase computation from whole WB ITD and IPD for frequency bins with 
* no transmitted individual IPD
**************************************************************************/
static void calc_Phase_syn_ITD(Word16 nb, Word16 iAngleStep, 
                               Word16 iAangleRotate, Word16 *mono_dec_real, 
                               Word16 *mono_dec_imag, Word16 *c, 
                               const Word16 *ptr_cidx, Word16 *L_mag, Word16 *R_mag, 
                               Word16 *L_real_syn, Word16 *L_imag_syn, 
                               Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 j; 
    Word16 ipd_diff_q;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR(j = 0; j< nb; j++)
    {
        ipd_diff_q = Round_Phase(iAangleRotate);
        iAangleRotate = sub(iAangleRotate ,iAngleStep ); // Q12
        Phase_syn_ITD(ipd_diff_q, mono_dec_real[j], mono_dec_imag[j], c[ptr_cidx[j]], L_mag[j], R_mag[j],
                      &L_real_syn[j], &L_imag_syn[j], &R_real_syn[j], &R_imag_syn[j]);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_Phase_syn_ITD0
*
* phase synthesis case swb_mono_flag = 0
**************************************************************************/
static void calc_Phase_syn_ITD0(Word16 nb, Word16 *mono_dec_real, 
                               Word16 *mono_dec_imag, Word16 *c, 
                               const Word16 *ptr_cidx, Word16 *L_mag, Word16 *R_mag, 
                               Word16 *L_real_syn, Word16 *L_imag_syn, 
                               Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 j; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR(j = 0; j< nb; j++)
    {
        Phase_syn_ITD0(mono_dec_real[j], mono_dec_imag[j], c[ptr_cidx[j]], L_mag[j], R_mag[j],
                       &L_real_syn[j], &L_imag_syn[j], &R_real_syn[j], &R_imag_syn[j]);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantIPD
*
* decoding of all quantized IPDs
**************************************************************************/
static void dequantIPD(Word16 mode, Word16 SWB_WB_flag, Word16 *bpt_stereo, Word16 fb_ipd, 
                       Word16 *pre_ipd_q, Word16 stereo_mono_flag, 
                       Word16 swb_ILD_mode,  Word16 *swb_frame_idx,
                       Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                       Word16 *c, Word16 *L_mag, Word16 *R_mag, 
                       Word16 *L_real_syn, Word16 *L_imag_syn, 
                       Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 nbBand, nbBit, ipd_diff_q, IPD_q, idx;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((5) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    /* if WB IPD from bin 2 to 9 quantized with 5 bits 
        otherwise IPD from bin 2 to 7 quantized with 5 bits */
    nbBand = sub(sub(IPD_SYN_WB_END_WB+1, IPD_SYN_START), shl(SWB_WB_flag,1) );
    IF(stereo_mono_flag != 0)
    {
        dequantIPD_5bits(nbBand, bpt_stereo, fb_ipd, &pre_ipd_q[IPD_SYN_START], 
                         &mono_dec_real[IPD_SYN_START], &mono_dec_imag[IPD_SYN_START], 
                         &c_idx[IPD_SYN_START], c, &L_mag[IPD_SYN_START], &R_mag[IPD_SYN_START],
                         &L_real_syn[IPD_SYN_START], &L_imag_syn[IPD_SYN_START], 
                         &R_real_syn[IPD_SYN_START], &R_imag_syn[IPD_SYN_START]);
    }
    ELSE
    {
        dequantIPD_5bits_0(nbBand, bpt_stereo, fb_ipd, &pre_ipd_q[IPD_SYN_START], 
                           &mono_dec_real[IPD_SYN_START], &mono_dec_imag[IPD_SYN_START], 
                           &c_idx[IPD_SYN_START], c, &L_mag[IPD_SYN_START], &R_mag[IPD_SYN_START],
                           &L_real_syn[IPD_SYN_START], &L_imag_syn[IPD_SYN_START], 
                           &R_real_syn[IPD_SYN_START], &R_imag_syn[IPD_SYN_START]);
    }

    nbBit = shl(5,3); /* if WB IPD from bin 2 to 9 quantized with 5 bits => 40 bits (8*5 ) */ 
    IF(SWB_WB_flag !=0)
    {   /* SWB case */
        nbBit = sub(nbBit,10);
        /*IPD synthesis for SWB stereo*/
        bpt_stereo += nbBit;
        /* decode IPD for bin 8 */
        IPD_q = pre_ipd_q[IPD_SYN_WB_END_SWB];move16();
        /*if swb_ILD_mode =0  IPD[8] quantized on 4 bits , next  bit frame_idx 
        else swb_ILD_mode =1 IPD[8] quantized on 5 bits */
        read_index5( bpt_stereo, &idx);
        bpt_stereo += 5;
        nbBit = add(nbBit,5);
        pre_ipd_q[IPD_SYN_WB_END_SWB] = tab_phase_q5[idx];move16();
        IF(swb_ILD_mode == 0)
        {
            *swb_frame_idx = s_and(idx, 0x1); move16();
            idx = shr(idx,1);
            pre_ipd_q[IPD_SYN_WB_END_SWB] = tab_phase_q4[idx];move16();
        }
        ipd_diff_q = sub(IPD_q, fb_ipd );
        ipd_diff_q = Round_Phase(ipd_diff_q);

        if(stereo_mono_flag == 0)
        {
            ipd_diff_q = 0; move16();
        }
        Phase_syn_IPD(ipd_diff_q, IPD_q, mono_dec_real[IPD_SYN_WB_END_SWB], mono_dec_imag[IPD_SYN_WB_END_SWB],
                      c[c_idx[IPD_SYN_WB_END_SWB]], L_mag[IPD_SYN_WB_END_SWB], R_mag[IPD_SYN_WB_END_SWB],
                      &L_real_syn[IPD_SYN_WB_END_SWB], &L_imag_syn[IPD_SYN_WB_END_SWB],
                      &R_real_syn[IPD_SYN_WB_END_SWB], &R_imag_syn[IPD_SYN_WB_END_SWB]);
        IF(sub(mode, MODE_R5ss) == 0)
        { /* IPD synthesis for R5ss only decode 16 IPDs bins 9 to 24 each quantized with 5 bits*/
            bpt_stereo += 5;   /* skip SHB ILD in SL1 to read SL2*/ 
            nbBand = sub(IPD_SYN_SWB_END_SWB+1, IPD_SYN_WB_END_SWB + 1);
            dequantIPD_5bits(nbBand, bpt_stereo, fb_ipd, &pre_ipd_q[IPD_SYN_WB_END_SWB + 1], 
                             &mono_dec_real[IPD_SYN_WB_END_SWB + 1], &mono_dec_imag[IPD_SYN_WB_END_SWB + 1], 
                             &c_idx[IPD_SYN_WB_END_SWB + 1], c,
                             &L_mag[IPD_SYN_WB_END_SWB + 1], &R_mag[IPD_SYN_WB_END_SWB + 1],
                             &L_real_syn[IPD_SYN_WB_END_SWB + 1], &L_imag_syn[IPD_SYN_WB_END_SWB + 1], 
                             &R_real_syn[IPD_SYN_WB_END_SWB + 1], &R_imag_syn[IPD_SYN_WB_END_SWB + 1]);
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantIPD_5bits
*
* decoding and synthesis for nb consecutive bins
**************************************************************************/
static void dequantIPD_5bits(Word16 nb, Word16 *bpt_stereo, Word16 fb_ipd, 
                             Word16 *pre_ipd_q, Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                             const Word16 *c_idx, Word16 *c, 
                             Word16 *L_mag, Word16 *R_mag, 
                             Word16 *L_real_syn, Word16 *L_imag_syn, 
                             Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 b, j, val;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    FOR(b=0; b<nb; b++)
    {
        j = c_idx[b]; move16();
        val = c[j]; move16();
        dequantOneIPD_5bits(bpt_stereo, fb_ipd, &pre_ipd_q[b], 
                            mono_dec_real[b], mono_dec_imag[b], val, L_mag[b], R_mag[b],
                            &L_real_syn[b], &L_imag_syn[b], &R_real_syn[b], &R_imag_syn[b]);
        bpt_stereo += 5;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantIPD_5bits_0
*
* decoding and synthesis for nb consecutive bins case stereo_mono_flag = 0
**************************************************************************/
static void dequantIPD_5bits_0(Word16 nb, Word16 *bpt_stereo, Word16 fb_ipd, 
                               Word16 *pre_ipd_q, Word16 *mono_dec_real, Word16 *mono_dec_imag, 
                               const Word16 *c_idx, Word16 *c, 
                               Word16 *L_mag, Word16 *R_mag, 
                               Word16 *L_real_syn, Word16 *L_imag_syn, 
                               Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 b, j, val;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    FOR(b=0; b<nb; b++)
    {
        j = c_idx[b]; move16();
        val = c[j]; move16();
        dequantOneIPD_5bits_0(bpt_stereo, fb_ipd, &pre_ipd_q[b], mono_dec_real[b], mono_dec_imag[b], 
                              val, L_mag[b], R_mag[b], &L_real_syn[b], &L_imag_syn[b], 
                              &R_real_syn[b], &R_imag_syn[b]);
        bpt_stereo += 5;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantOneIPD_5bits
*
* decoding and synthesis for one bin
**************************************************************************/
static void dequantOneIPD_5bits(Word16 *bpt_stereo, Word16 fb_ipd, 
                                Word16 *pre_ipd_q, Word16 mono_dec_real, Word16 mono_dec_imag, 
                                Word16 c, Word16 L_mag, Word16 R_mag, 
                                Word16 *L_real_syn, Word16 *L_imag_syn, 
                                Word16 *R_real_syn, Word16 *R_imag_syn)

{
    Word16 idx, IPD_q, ipd_diff_q ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    read_index5( bpt_stereo, &idx);
    IPD_q = *pre_ipd_q ; move16();
    *pre_ipd_q = tab_phase_q5[idx];move16();
    ipd_diff_q = sub(IPD_q, fb_ipd );
    ipd_diff_q = Round_Phase(ipd_diff_q);
    Phase_syn_IPD(ipd_diff_q, IPD_q, mono_dec_real, mono_dec_imag, c, L_mag, R_mag, 
                  L_real_syn, L_imag_syn, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantOneIPD_5bits_0
*
* decoding and synthesis for one bin case stereo_mono_flag = 0
**************************************************************************/
static void dequantOneIPD_5bits_0(Word16 *bpt_stereo, Word16 fb_ipd, 
                                  Word16 *pre_ipd_q, Word16 mono_dec_real, Word16 mono_dec_imag, 
                                  Word16 c, Word16 L_mag, Word16 R_mag, 
                                  Word16 *L_real_syn, Word16 *L_imag_syn, 
                                  Word16 *R_real_syn, Word16 *R_imag_syn)
{
    Word16 idx, IPD_q ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    read_index5(bpt_stereo, &idx);
    IPD_q      = *pre_ipd_q ; move16();
    *pre_ipd_q = tab_phase_q5[idx];move16();

    Phase_syn_IPD0(IPD_q, mono_dec_real, mono_dec_imag, c, L_mag, R_mag, 
                   L_real_syn, L_imag_syn, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* postProcStereo
*
* apply postprocessing to n point
**************************************************************************/
static void postProcStereo(Word16 n, Word16 *gainPost, Word16 *monoReal, Word16 *monoImag) 
{
    Word16 i;
    Word16 *ptrGain, *ptrR, *ptrI;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (0) * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    ptrGain = gainPost;
    ptrR = monoReal;
    ptrI = monoImag;
    FOR(i=0; i<n; i++)
    {
        boundPostProc(*ptrGain, ptrR, ptrI);
        ptrGain++; ptrR++; ptrI++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* boundPostProc
*
* apply postprocessing to one point
**************************************************************************/
static void boundPostProc(Word16 boundPost, Word16 *monoReal, Word16 *monoImag) 
{
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((0) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    IF (sub(boundPost, 18842) < 0)
    {
        *monoReal= round_fx_L_shl_L_mult(*monoReal, boundPost, 1); move16(); /* Q(q_mono) */
        *monoImag= round_fx_L_shl_L_mult(*monoImag, boundPost, 1); move16(); /* Q(q_mono) */
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantILD0
*
* decoded sub band ILDs with regular subband increment (+2 )
**************************************************************************/
static void dequantILD0(Word16 nb4, Word16 nb3, Word16 *mem_ILD_q, Word16 *r1ws_pt)
{
    Word16 idx, b;
    Word16 *ptrILD, *ptrILDmem; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif

    /*dequantize ILD in each sub-band*/
    ptrILD    = mem_ILD_q;
    ptrILDmem = ptrILD; 
    
    read_index5(r1ws_pt, &idx);
    r1ws_pt += 5; 
    *ptrILD = tab_ild_q5[idx]; move16();
    ptrILD += 2;
    /* Differential quantization of following ILD */
    FOR(b = 0; b < nb4; b++)
    {
        read_index4(r1ws_pt, &idx);
        r1ws_pt += 4;
        *ptrILD = add(*ptrILDmem, tab_ild_q4[idx]); move16();
        ptrILD += 2;
        ptrILDmem += 2;
    }
    FOR(b=0; b < nb3; b ++)
    {
        read_index3(r1ws_pt, &idx);
        r1ws_pt += 3;
        *ptrILD = add(*ptrILDmem, tab_ild_q3[idx]); move16();
        ptrILD += 2;
        ptrILDmem += 2;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantILD
*
* decoded sub band ILDs with irregular subband increment
**************************************************************************/
static void dequantILD(Word16 frame_idx, Word16 *mem_ILD_q, Word16 *r1ws_pt)
{
    Word16 b, preILDq, idx;
    const Word16 *ptrBand;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ( 3 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif

    ptrBand = &band_index[frame_idx][0];
    /* 1st band in category absolute quantization */
    b = *ptrBand++; move16();
    read_index5( r1ws_pt, &idx);
    preILDq      = tab_ild_q5[idx]; move16();
    mem_ILD_q[b] = preILDq ; move16();
    r1ws_pt += 5;   

    /* 2nd band in category  differential quantization  */
    b = *ptrBand++; move16();
    read_index4( r1ws_pt, &idx);
    preILDq      = add(preILDq , tab_ild_q4[idx]); move16();
    mem_ILD_q[b] = preILDq; move16();
    r1ws_pt += 4;  
                
    b = *ptrBand++; move16();
    IF(sub(frame_idx,2) < 0) 
    { /* frame_idx= 0 or 1 */
        /* 3rd band in category  differential quantization  */
        read_index4( r1ws_pt, &idx);
        preILDq      = add(preILDq , tab_ild_q4[idx]); move16();
        mem_ILD_q[b] = preILDq ; move16();
        r1ws_pt += 4;

        /*4th band in category absolute quantization */
        b = *ptrBand++; move16();
        read_index5( r1ws_pt, &idx);
        preILDq      = tab_ild_q5[idx]; move16();
        mem_ILD_q[b] = preILDq ; move16();
        r1ws_pt += 5;
    }
    ELSE
    {        /* frame_idx= 2 or 3 */
        /* 3rd band in category  absolute quantization  */
        read_index5( r1ws_pt, &idx);
        preILDq      = tab_ild_q5[idx]; move16();
        mem_ILD_q[b] = preILDq ; move16();
        r1ws_pt += 5;

        /*4th band in category differential  quantization */
        b = *ptrBand++; move16();
        read_index4( r1ws_pt, &idx);
        preILDq      = add(preILDq , tab_ild_q4[idx]); move16();
        mem_ILD_q[b] = preILDq ; move16();
        r1ws_pt += 4;
    }

    b = *ptrBand++; move16();
    /* last band in category  differential quantization  */
    read_index4( r1ws_pt, &idx);
    preILDq      = add(preILDq , tab_ild_q4[idx]); move16();
    mem_ILD_q[b] = preILDq ; move16();
    r1ws_pt += 4;
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* dequantRefineILD
*
* dequantization of ILD refinement
**************************************************************************/
static Word16 dequantRefineILD(Word16 swb_flag, Word16 ic_flag, Word16 frame_idx, Word16 idx, 
                               Word16 *mem_ILD_q, Word16 *r1ws_pt)
{
    Word16 nbBitRefineILD, i;
    Word16 parityFrame_idx, parityIdx, flagNbBand, incBand, b0;
    Word16 idx1;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((8) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    /* case 4* Frame_idx + idx */
    i = shl(frame_idx, 2);
    i = add(i, idx);
    incBand = 1; move16(); /* increment index subband 1  case parityFrame_idx !=parityIdx*/
    flagNbBand = 0; move16(); /* =0 if 3 sub band to be quantized Frame_idx != idx, otherwise 2 subbands */

    parityFrame_idx = s_and(frame_idx, 0x01);
    parityIdx = s_and(idx, 0x01);
    if(sub(parityFrame_idx,  parityIdx ) == 0)
    {
        incBand = add(incBand,1); /* increment index subband 2 case parityFrame_idx= parityIdx*/
    }
    if(sub(frame_idx,  idx ) == 0)
    {
        flagNbBand = add(flagNbBand,1); /* only 2 subband to quantize */
    }
    b0 = startBand[i]; move16(); /* 1st subband to be quantized */

    /*The number of bits used for ILD refinement is determined by IC flag*/
    IF(ic_flag)
    { /* case ic transmitted */
        nbBitRefineILD = 4; move16();
        IF(swb_flag == 0)
        { /* WB and case ic transmitted */
            nbBitRefineILD = add(nbBitRefineILD, 1);
            /* only 2 subbands to quantize on 3 and 2 bits */ 
            read_index3(r1ws_pt, &idx1);
            r1ws_pt += 3;
            mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q3[idx1]); move16();

            b0 = add(b0, incBand);
            read_index2(r1ws_pt, &idx1);
            r1ws_pt += 2;
            mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q2[idx1]); move16();
        } /*     end WB and case ic transmitted */
        ELSE
        {   /* SWB and case ic transmitted */
            /* only 2 subbands to quantize both on 2 bits */ 
            read_index2(r1ws_pt, &idx1);
            r1ws_pt += 2;
            mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q2[idx1]); move16();

            b0 = add(b0, incBand);
            read_index2(r1ws_pt, &idx1);
            r1ws_pt += 2;
            mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q2[idx1]); move16();
        } /* end SWB and case ic transmitted */
    } /* end case ic transmitted */
    ELSE
    { /* case ic non transmitted */
        nbBitRefineILD = 6; move16();

        IF (swb_flag == 0)
        { /* WB and case ic non transmitted */
            nbBitRefineILD = add(nbBitRefineILD,1);
            IF( flagNbBand == 0)
            {  /* case 3 subbands quantized first on 3 bits , last two subbands on 2 bits */
                read_index3(r1ws_pt, &idx1);
                r1ws_pt += 3;
                mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q3[idx1]); move16();
                FOR(i = 0; i < 2; i++)
                {
                    b0 = add(b0, incBand);
                    read_index2(r1ws_pt, &idx1);
                    r1ws_pt += 2;
                    mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q2[idx1]); move16();
                }
            } /* end case 3 subbands quantized first on 3 bits , last two subbands on 2 bits */
            ELSE
            { /* case 2 subbands quantized first on 4 bits, last on 3 bits */
                read_index4(r1ws_pt, &idx1);
                r1ws_pt += 4;
                mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q4[idx1]); move16();

                b0 = add(b0, incBand);
                read_index3(r1ws_pt, &idx1);
                r1ws_pt += 3;
                mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q3[idx1]); move16();
            } /* end case 2 subbands quantized first on 4 bits, last on 3 bits */
        } /* end  WB and case ic non transmitted */
        ELSE
        { /* SWB and case ic non transmitted */
            IF( flagNbBand == 0)
            {  /* case 3 subband quantized all on 2 bits */
                FOR(i = 0; i < 3; i++)
                {
                    read_index2(r1ws_pt, &idx1);
                    r1ws_pt += 2;
                    mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q2[idx1]); move16();
                    b0 = add(b0, incBand);
                }
            } /* end case 3 subband quantized all on 2 bits */
            ELSE
            { /* case 2 subband quantized all on 3 bits */
                FOR(i = 0; i < 2; i++)
                {
                    read_index3(r1ws_pt, &idx1);
                    r1ws_pt += 3;
                    mem_ILD_q[b0] = add(mem_ILD_q[b0], tab_ild_q3[idx1]); move16();
                    b0 = add(b0, incBand);
                }
            } /* end case 2 subband quantized all on 3 bits */
        } /* end SWB and case ic non transmitted */
    } /* end case ic non transmitted */

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (nbBitRefineILD );
}

/*************************************************************************
* calc_ICSyntScaleFactors
*
* calculate the scale factors for Inter Channel Coherence Synthesis
**************************************************************************/
static void calc_ICSyntScaleFactors(Word16 n, Word16 *pre_ild_q, Word16 icSq, 
                                    Word16 *w1_d_c1, Word16 *w2_d_c2, Word16 *w3)
{
    Word16 i;
    Word16 preILD, relPow;
    Word16 *ptrILDq;
    Word16 *ptr1, *ptr2, *ptr3;
    const Word16 *ptr_c_table10_1, *ptr_c_d_table_factor;
    const Word32 *ptr_c_d_table_factor_d_c1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (0) * SIZE_Word32 + 7 * SIZE_Ptr), "dummy");
#endif

    ptr1 = w1_d_c1;
    ptr2 = w2_d_c2;
    ptr3 = w3;

    ptr_c_table10_1 = &c_table10_1[110];
    ptr_c_d_table_factor_d_c1 = &c_d_table_factor_d_c1[110];
    ptr_c_d_table_factor = &c_d_table_factor[110];
    ptrILDq = pre_ild_q;

    FOR(i = 0; i < n; i++)
    {
        preILD  = *ptrILDq++; move16();
        relPow  = calcRelPowerLR(icSq, preILD); move16();
        *ptr1++ = calcScaleFacCorrel(ptr_c_table10_1[preILD], ptr_c_d_table_factor_d_c1[preILD], relPow); move16();
        *ptr3++ = calcScaleFacUncorrel(ptr_c_d_table_factor[preILD], relPow); move16();
        preILD  = negate(preILD);
        *ptr2++ = calcScaleFacCorrel(ptr_c_table10_1[preILD], ptr_c_d_table_factor_d_c1[preILD], relPow); move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calcRelPowerLR
*
* calculate the relative power of left and right channels
**************************************************************************/
static Word16 calcRelPowerLR(Word16 icSq, Word16 preILD)
{
    Word16 valp, valn;
    const Word16 *ptr_c_table10_1;
    Word32 tmp32;
    Word16 tmp16;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (1) * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    ptr_c_table10_1 = &c_table10_1[110];
    valp  = ptr_c_table10_1[preILD]; move16();
    valn  = ptr_c_table10_1[negate(preILD)]; move16();
    tmp32 = L_mult0(valp, valn);
    tmp32 = L_mls(tmp32, icSq);
    tmp32 = L_shl(tmp32, 3);
    tmp32 = L_sub(2147483647, tmp32);
    tmp16 = L_sqrt(tmp32);
    tmp16 = sub(32767, tmp16);
    tmp16 = shr(tmp16, 1); move16();

    tmp16 = s_min(tmp16, valp);
    tmp16 = s_min(tmp16, valn);
    tmp16 = s_max(tmp16, 0);
    move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(tmp16);
}

/*************************************************************************
* calcScaleFacCorrel
*
* calculate a scale factors for one bin correlated signal
**************************************************************************/
static Word16 calcScaleFacCorrel(Word16 val1, Word32 Lval2, Word16 val3)
{
    Word16 tmp16;
    Word32 tmp32, tmp32_2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (2) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp16 = sub(val1, val3);
    tmp32_2 = L_mult0(tmp16, 16384);
    tmp16 = L_sqrt(tmp32_2);
    tmp32 = L_mls(Lval2, tmp16); /* q29 */;
    tmp16 = shl(extract_l(tmp32), 3); /* q14 */
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(tmp16);
}

/*************************************************************************
* calcScaleFacUncorrel
*
* calculate a scale factors for one bin uncorrelated signal
**************************************************************************/
static Word16 calcScaleFacUncorrel(Word16 val1, Word16 val3)
{
    Word16 tmp16;
    Word32 tmp32, tmp32_2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (2) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp32_2 = L_mult0(val3, 16384);
    tmp16 = L_sqrt(tmp32_2);
    tmp32 = L_mult(tmp16, val1); /* q29 */
    tmp16 = round_fx(L_shl(tmp32,1));/* q14 */
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(tmp16);
}

/*************************************************************************
* decorrel
*
* de-correlated signals are added to the left and right channel respectively
**************************************************************************/
static void decorrel(Word16 n, Word16 region_ic , Word16 *w3, 
                     Word16 *w1_d_c1, Word16 *w2_d_c2,
                     Word16 *memDecorrReal, Word16 *memDecorrImag, 
                     Word16 *L_real_syn, Word16 *L_imag_syn, Word16 q_left,
                     Word16 *R_real_syn, Word16 *R_imag_syn, Word16 q_right)
{
    Word16 i, j, tmp16;
    Word16 *ptrMemR0, *ptrMemI0, *ptrMemR1, *ptrMemI1;
    Word16 *ptr_l_real, *ptr_l_imag, *ptr_r_real, *ptr_r_imag ;
    const Word16 *ptr_c_idx;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((3) * SIZE_Word16 +  (0) * SIZE_Word32 + 9 * SIZE_Ptr), "dummy");
#endif
    ptrMemR0 = memDecorrReal + region_ic;
    ptrMemI0 = memDecorrImag + region_ic;
    ptrMemR1 = ptrMemR0 + (NFFT/2 + 1) * 2;
    ptrMemI1 = ptrMemI0 + (NFFT/2 + 1) * 2;

    ptr_l_real = L_real_syn+region_ic;
    ptr_l_imag = L_imag_syn+region_ic;
    ptr_r_real = R_real_syn+region_ic;
    ptr_r_imag = R_imag_syn+region_ic;

    ptr_c_idx = c_idx+region_ic;
    FOR(i=0; i<n; i++)
    {
        j = *ptr_c_idx++; move16();
        tmp16 = decorrelOnePoint(*ptrMemR0++, *ptr_l_real, w3[j], w1_d_c1[j], q_left); move16();
        *ptr_l_real++ = tmp16; move16();
        tmp16 = decorrelOnePoint(*ptrMemI0++, *ptr_l_imag, w3[j], w1_d_c1[j], q_left); move16();
        *ptr_l_imag++ = tmp16; move16();
        tmp16 = decorrelOnePoint(*ptrMemR1++, *ptr_r_real, w3[j], w2_d_c2[j], q_right); move16();
        *ptr_r_real++ = tmp16; move16();
        tmp16 = decorrelOnePoint(*ptrMemI1++, *ptr_r_imag,  w3[j], w2_d_c2[j], q_right); move16();
        *ptr_r_imag++ = tmp16; move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* decorrelOnePoint
*
* de-correlation for one frequency bin
**************************************************************************/
static Word16 decorrelOnePoint(Word16 mem, Word16 val_io, Word16 val3, Word16 val1, Word16 qVal)
{
    Word16 tmp16;
    Word32 tmp32;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (1) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp16 = mult_r(mem, val3);
    tmp32 = L_shr(L_mult0(val1, val_io), 14);
    tmp32 = L_mac0(tmp32, shl(tmp16, qVal), 2); 
    val_io = extract_l(tmp32); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (val_io);
}

/*************************************************************************
* stereo_dec_timepos
*
* Super higher band stereo post processing
**************************************************************************/
Word16 stereo_dec_timepos(Word16  sig_Mode,
                          Word16* sTenv_SWB,
                          Word16* sIn_Out,      /* (i/o): time domain signal */
                          void*   work,         /* (i/o): Pointer to work space */
                          Word16  T_modify_flag,
                          Word16  channel_flag,
                          Word16  delay,
                          Word16  ratio_s
                          )
{
    BWE_state_dec *dec_st = (BWE_state_dec *)work;
    Word16 i, pos;
    Word16 pos_s;
    Word16 *pit_s;
    Word16 atteu;
    Word16 *tPres;
    Word16 enn_hi;
    Word16 enn_lo;

    Word16 tmp_Tenv_SWB_s[SWB_TENV];
    Word32 enn_s;
    Word32 ener_prev;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((7 + SWB_TENV) * SIZE_Word16 
        +  2 * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    tPres = dec_st->right_tPre_s;/*right channel*/    
    if(sub(channel_flag, 1) == 0) 
    {
        tPres = dec_st->left_tPre_s;/*left channel*/
    }

    pit_s = sIn_Out;

    FOR (i = 0; i < SWB_TENV; i++)
    {
        enn_s  = L_mac0_Array(SWB_TENV_WIDTH, pit_s , pit_s );
        pit_s += SWB_TENV_WIDTH;
        enn_s  = L_shr(enn_s,4);
        enn_lo = L_Extract_lc( enn_s, &enn_hi);
        enn_s  = Mpy_32_16( enn_hi, enn_lo, 26214 ); 
        enn_s  = Inv_sqrt(enn_s); /* Q(30) */
        tmp_Tenv_SWB_s[i] = extract_l(L_mult0(round_fx(enn_s),sTenv_SWB[i])); move16();//Q14
    }

    /*Calculate the time evelope of the input signal*/
    FOR (pos = 0; pos < delay; pos++)
    {
        sIn_Out[pos] = extract_l(L_shr(L_mult0(tmp_Tenv_SWB_s[0],sIn_Out[pos]),14)); move16();
    }
    FOR (i = 0; i < SWB_TENV - 1; i++)
    {
        FOR (pos = 0; pos < SWB_TENV_WIDTH; pos++)
        {
            sIn_Out[i * SWB_TENV_WIDTH + delay + pos] = extract_l(L_shr(L_mult0(tmp_Tenv_SWB_s[i],sIn_Out[i * SWB_TENV_WIDTH + delay + pos]),14)); move16();
        }
    }
    FOR (pos = 0; pos < SWB_TENV_WIDTH - delay; pos++)
    {
        sIn_Out[(SWB_TENV - 1) * SWB_TENV_WIDTH + delay + pos] = extract_l(L_shr(L_mult0(tmp_Tenv_SWB_s[SWB_TENV - 1],sIn_Out[(SWB_TENV - 1) * SWB_TENV_WIDTH + delay + pos]),14)); move16();
    }

    IF (sub(T_modify_flag, 1) == 0)
    {
        MaxArray(SWB_TENV, sTenv_SWB, &pos_s);
        pit_s = sIn_Out;
        IF (pos_s != 0)
        {
            pit_s = &sIn_Out[SUB_SWB_T_WIDTH * pos_s + delay];
            atteu = div_s(sTenv_SWB[pos_s - 1] , sTenv_SWB[pos_s]);
            array_oper(HALF_SUB_SWB_T_WIDTH, atteu, pit_s, pit_s, &mult);
        }
        ELSE
        {
            pit_s = tPres;
            ener_prev = L_mac0_Array(HALF_SUB_SWB_T_WIDTH, pit_s, pit_s);
            ener_prev = L_mls(ener_prev,3277); /* divide 10 */
            SqrtI31(ener_prev,&ener_prev);
            atteu = div_l( ener_prev , sTenv_SWB[pos_s]);
            pit_s = sIn_Out;
            array_oper(HALF_SUB_SWB_T_WIDTH + delay, atteu, pit_s, pit_s, &mult);
        }
    }
    test();

    FOR(pos = 0; pos < SWB_T_WIDTH; pos++)
    {
        sIn_Out[pos] = shl(mult(sIn_Out[pos],ratio_s),1); move16();
    }

    FOR(i=0; i<SWB_T_WIDTH; i+=2)
    {
        sIn_Out[i] = negate(sIn_Out[i]); move16();
    }

    mov16( HALF_SUB_SWB_T_WIDTH, &sIn_Out[SWB_T_WIDTH-HALF_SUB_SWB_T_WIDTH], tPres );
    dec_st->pre_tEnv = sTenv_SWB[3]; move16();
    dec_st->pre_mode = sig_Mode; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return 2;
}

/*************************************************************************
* g722_stereo_decoder_shb
*
* G722 super higher band stereo decoder
**************************************************************************/
void g722_stereo_decoder_shb(Word16* bpt_stereo_swb, 
                             Word16* coef_SWB_s, 
                             Word16  coef_q,
                             Word16* syn_left_swb_s,
                             Word16* syn_right_swb_s,
                             void*   ptr,
                             Word16  ploss_status,
                             Word16  Mode
                             )
{
    Word16* bpt = bpt_stereo_swb; 
    g722_stereo_decode_WORK *w = (g722_stereo_decode_WORK *)ptr; 
    Word16 i;
    Word16 left_mdct_s[L_FRAME_WB],right_mdct_s[L_FRAME_WB];
    Word16 idx;
    Word16 q_left,q_right;
    Word16 mono_mdct[80];
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((5 + 3 * 80) * SIZE_Word16 +  0 * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif
    IF(ploss_status == 0)
    {
        IF(w->swb_ILD_mode)
        {
            read_index5( bpt, &idx);
            bpt += 5;
            tmp = shr(tab_ild_q5[idx],9);
            w->c1_swb[0] = c_table20[80 + tmp]; move16();
            w->c2_swb[0] = c_table20[80 - tmp]; move16();
            w->c1_swb[1] = w->c1_swb[0]; move16();
            w->c2_swb[1] = w->c2_swb[0]; move16();
        }
        ELSE
        {
            read_index5( bpt, &idx);
            bpt += 5;
            tmp = shr(tab_ild_q5[idx],9);
            w->c1_swb[w->swb_frame_idx] = c_table20[80 + tmp]; move16();
            w->c2_swb[w->swb_frame_idx] = c_table20[80 - tmp]; move16();
        }
    }
    FOR (i = 0; i < 60; i++) 
    {
        mono_mdct[i] = mult(coef_SWB_s[i],29309); move16();//Q20 1/(16.0f * Sqrt(5.0f))
    }

    coef_q = add(coef_q,5);

    /* L and R channels reconstructed */
    FOR(i=swb_bands[0]; i<swb_bands[1]; i++)
    {
        left_mdct_s[i]  = mult(w->c1_swb[0],mono_mdct[i]); move16();
        right_mdct_s[i] = mult(w->c2_swb[0],mono_mdct[i]); move16();
    }
    FOR(i=swb_bands[1]; i<swb_bands[2]; i++)
    {
        left_mdct_s[i]  = mult(w->c1_swb[1],mono_mdct[i]); move16();
        right_mdct_s[i] = mult(w->c2_swb[1],mono_mdct[i]); move16();
    }

    q_left = sub(coef_q,2);
    q_right = sub(coef_q,2);

    zero16(20, &left_mdct_s[60]);
    zero16(20, &right_mdct_s[60]);

    PCMSWB_TDAC_inv_mdct(syn_left_swb_s,  left_mdct_s,  w->mem_left_mdct,  q_left, 
                         &w->pre_norm_left,  (Word16) 0, w->sCurSave_left);
    PCMSWB_TDAC_inv_mdct(syn_right_swb_s, right_mdct_s, w->mem_right_mdct, q_right,
                         &w->pre_norm_right, (Word16) 0, w->sCurSave_right);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* Syn_FFT_bin
*
* FFT bin synthesis based on the amplitude and the phase
**************************************************************************/
static void Syn_FFT_bin(Word16 iRPhase,
                        Word16 mag,
                        Word16 *real,
                        Word16 *imag)
{
    Word16 iPhaseCos,iPhaseSin,iTmpPhase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    iPhaseCos = spx_cos(iRPhase)  ; // Q15
    iTmpPhase = sub( PID2_FQ12 , iRPhase ) ;
    iPhaseSin = spx_cos(iTmpPhase); // Q15
    *real = mult(mag,iPhaseCos);
    *imag = mult(mag,iPhaseSin);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* Phase_syn_ITD
*
* left and right channel signals synthesis based on the whole wideband ITD 
**************************************************************************/
void Phase_syn_ITD(Word16 ipd_diff_q,
                   Word16 mono_dec_real,
                   Word16 mono_dec_imag,
                   Word16 c,
                   Word16 L_mag,
                   Word16 R_mag,
                   Word16 *L_real_syn,
                   Word16 *L_imag_syn,
                   Word16 *R_real_syn,
                   Word16 *R_imag_syn
                   )
{
    Word16 M_phase;
    Word16 iCPhase,iC1Phase,iRPhase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    M_phase = arctan2_fix32(L_deposit_l(mono_dec_imag), L_deposit_l(mono_dec_real));

    iCPhase = mult(c,  ipd_diff_q);
    iRPhase = add(M_phase, iCPhase );
    Syn_FFT_bin(iRPhase, L_mag, L_real_syn, L_imag_syn);

    iC1Phase = sub( ipd_diff_q , iCPhase );
    iRPhase = sub(M_phase, iC1Phase );
    Syn_FFT_bin(iRPhase, R_mag, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}
/*************************************************************************
* Phase_syn_ITD0  case stereo_mono_flag = 0
* left and right channel signals synthesis based on the whole wideband ITD 
* as ipd_diff_q =0 then iCPhase = iC1Phase= 0; iRPhase = M_phase; 
*
**************************************************************************/
void Phase_syn_ITD0(Word16 mono_dec_real,
                    Word16 mono_dec_imag,
                    Word16 c,
                    Word16 L_mag,
                    Word16 R_mag,
                    Word16 *L_real_syn,
                    Word16 *L_imag_syn,
                    Word16 *R_real_syn,
                    Word16 *R_imag_syn
                    )
{
    Word16 M_phase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    M_phase = arctan2_fix32(L_deposit_l(mono_dec_imag), L_deposit_l(mono_dec_real));
    Syn_FFT_bin(M_phase, L_mag, L_real_syn, L_imag_syn);
    Syn_FFT_bin(M_phase, R_mag, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* Phase_syn_IPD
*
* left and right channel signals synthesis based on the whole wideband IPD
**************************************************************************/
void Phase_syn_IPD(Word16 ipd_diff_q,
                   Word16 IPD_q,
                   Word16 mono_dec_real,
                   Word16 mono_dec_imag,
                   Word16 c,
                   Word16 L_mag,
                   Word16 R_mag,
                   Word16 *L_real_syn,
                   Word16 *L_imag_syn,
                   Word16 *R_real_syn,
                   Word16 *R_imag_syn
                   )
{
    Word16 M_phase;
    Word16 iCPhase,iRPhase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    M_phase = arctan2_fix32(L_deposit_l(mono_dec_imag), L_deposit_l(mono_dec_real));

    iCPhase = mult(c,  ipd_diff_q);
    iRPhase = add(M_phase, iCPhase );
    Syn_FFT_bin(iRPhase, L_mag, L_real_syn, L_imag_syn);

    iRPhase = sub(Round_Phase(add(M_phase, iCPhase )),IPD_q);
    Syn_FFT_bin(iRPhase, R_mag, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* Phase_syn_IPD0
* 
* Phase synthesis case stereo_mono_flag = 0
* left and right channel signals synthesis based on the whole wideband IPD
* as ipd_diff_q =0 so iCPhase = 0; and
* left channel iRPhase = M_phase; 
* right channel iRPhase = sub(Round_Phase(M_phase),IPD_q);; 
**************************************************************************/
void Phase_syn_IPD0(Word16 IPD_q,
                    Word16 mono_dec_real,
                    Word16 mono_dec_imag,
                    Word16 c,
                    Word16 L_mag,
                    Word16 R_mag,
                    Word16 *L_real_syn,
                    Word16 *L_imag_syn,
                    Word16 *R_real_syn,
                    Word16 *R_imag_syn
                    )
{
    Word16 M_phase;
    Word16 iRPhase;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    M_phase = arctan2_fix32(L_deposit_l(mono_dec_imag), L_deposit_l(mono_dec_real));
    Syn_FFT_bin(M_phase, L_mag, L_real_syn, L_imag_syn);

    iRPhase = sub(Round_Phase(M_phase),IPD_q);
    Syn_FFT_bin(iRPhase, R_mag, R_real_syn, R_imag_syn);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* stereo_synthesis
*
* left and right channel signals synthesis based on the wideband ILD
**************************************************************************/
void stereo_synthesis(Word16* ILD_q,          /* i: ILD quantized */
                      Word16* mono_real,  /* i: mono signal */
                      Word16* mono_imag,
                      Word16  q_mono,
                      Word16* L_real_syn, /* o: L signal synthesis */
                      Word16* L_imag_syn, 
                      Word16* R_real_syn, /* o: R signal synthesis */
                      Word16* R_imag_syn,
                      Word16* q_left,
                      Word16* q_right,
                      Word16* L_mag,
                      Word16* R_mag,
                      Word16  ploss_status
                      )
{
    Word16   b, i;
    Word32 L_temp;
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    IF(ploss_status == 0)
    {
        FOR(i=0; i< IPD_SYN_START; i++)
        {
            L_real_syn[i] = mult(c_table20[80 + ILD_q[c_idx[i]]], mono_real[i]); move16();
            L_imag_syn[i] = mult(c_table20[80 + ILD_q[c_idx[i]]], mono_imag[i]); move16();
            R_real_syn[i] = mult(c_table20[80 - ILD_q[c_idx[i]]], mono_real[i]); move16();
            R_imag_syn[i] = mult(c_table20[80 - ILD_q[c_idx[i]]], mono_imag[i]); move16();
        }

        FOR(b=17; b<NB_SB; b++)
        {
            FOR(i=bands[b]; i<bands[b+1]; i++)
            {
                L_real_syn[i] = mult(c_table20[80 + ILD_q[b]], mono_real[i]); move16();
                L_imag_syn[i] = mult(c_table20[80 + ILD_q[b]], mono_imag[i]); move16();
                R_real_syn[i] = mult(c_table20[80 - ILD_q[b]], mono_real[i]); move16();
                R_imag_syn[i] = mult(c_table20[80 - ILD_q[b]], mono_imag[i]); move16();
            }
        }

        FOR(b=START_ILD; b<NB_SB; b++)
        {
            FOR(i=bands[b]; i<bands[b+1]; i++)
            {
                L_temp = L_mult( mono_real[i], mono_real[i]);
                L_temp = L_mac( L_temp , mono_imag[i] , mono_imag[i] ); // 2*q_left +1 
                SqrtI31( L_temp,  &L_temp );
                tmp = round_fx(L_temp);
                L_mag[i] = mult(c_table20[80 + ILD_q[b]],tmp); move16();
                R_mag[i] = mult(c_table20[80 - ILD_q[b]],tmp); move16();
            }
        }
    }
    ELSE
    {
        FOR(i=0; i< NFFT/2+1; i++)
        {
            L_real_syn[i] = mult(c_table20[80 + ILD_q[c_idx[i]]], mono_real[i]); move16();
            L_imag_syn[i] = mult(c_table20[80 + ILD_q[c_idx[i]]], mono_imag[i]); move16();
            R_real_syn[i] = mult(c_table20[80 - ILD_q[c_idx[i]]], mono_real[i]); move16();
            R_imag_syn[i] = mult(c_table20[80 - ILD_q[c_idx[i]]], mono_imag[i]); move16();
        }
    }
    *q_left = sub(q_mono,1);
    *q_right = sub(q_mono,1);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}
#endif /* LAYER_STEREO */
