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

/*************************************************************************
* calc_log_ener
*
* Calculate the Log2 of left and right energy for ILD calculation
**************************************************************************/
Word16 calc_log_ener(Word32 L_ener,Word32 R_ener,Word16 q_diff_en)
{
    Word16 tmp1,tmp2,tmp3;
    Word16 log2_exp_l,log2_frac_l;
    Word16 log2_exp_r,log2_frac_r;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (7 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    Log2(L_ener, &log2_exp_l, &log2_frac_l);
    Log2(R_ener, &log2_exp_r, &log2_frac_r);
    tmp1 = sub(sub( log2_exp_l, log2_exp_r), q_diff_en);
    tmp2 = sub(log2_frac_l, log2_frac_r);
    tmp3 = add(shl(tmp1,9),shr(tmp2,6));
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(tmp3);//Q9
}

/*************************************************************************
* ild_calc_dect
*
* Calculate the ILD in stereo transient detection function
**************************************************************************/
Word16 ild_calc_dect(Word32 L_ener,Word32 R_ener,Word16 q_diff_en)
{
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp = calc_log_ener(L_ener, R_ener, q_diff_en);
    tmp = shr(tmp,3);//Q6
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(mult(inv_LOG2_10,tmp));//Q4
}

/*************************************************************************
* ild_calculation
*
* Calculate the wideband ILD
**************************************************************************/
Word16 ild_calculation(Word32 L_ener,Word32 R_ener,Word16 q_left_en,Word16 q_right_en)
{
    Word16 tmp,tmp1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    tmp  = sub(q_left_en, q_right_en);
    tmp1 = calc_log_ener(L_ener, R_ener, tmp);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(round_fx(L_shl(L_mult(inv_LOG2_10,tmp1),2)));//Q9
}

/*************************************************************************
* ild_attack_detect
*
* Wideband stereo transient detection
**************************************************************************/
Word16 ild_attack_detect(Word32* L_ener, /* i: energy per sub-band of L channel */
                         Word32* R_ener, /* i: energy per sub-band of R channel */
                         void*   ptr,
                         Word16 *nbShL,
                         Word16 *nbShR
                         )
{
    Word16 b, i;
    Word16 ILD_sum, ILD_sumH;
    Word16 sum_mean,sum_meanH;
    Word16 dev,devH;

    Word16 flag;
    Word16 ILD;

    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (10 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    /*calculate ILD for each band*/
    ILD_sum = ild_calc_dect(L_ener[0],R_ener[0], sub(nbShL[0], nbShR[0]));
    FOR(b=1; b< 14; b++)
    {
        ILD = ild_calc_dect(L_ener[b],R_ener[b], sub(nbShL[b], nbShR[b]));

        ILD_sum = add(ILD_sum,ILD);//Q4
    }

    ILD_sumH = ild_calc_dect(L_ener[14],R_ener[14], sub(nbShL[14], nbShR[14]));
    FOR(b=15; b< 20; b++)
    {
        ILD = ild_calc_dect(L_ener[b],R_ener[b], sub(nbShL[b], nbShR[b]));
        ILD_sumH = add(ILD_sumH,ILD);//Q4
    }

    /*calculate the means of last FNUM frames*/

    w->pre_ild_sum[w->pos]   = ILD_sum;  move16();
    w->pre_ild_sum_H[w->pos] = ILD_sumH; move16();
    sum_mean  = add(w->pre_ild_sum[0], w->pre_ild_sum[1]);
    sum_meanH = add(w->pre_ild_sum_H[0], w->pre_ild_sum_H[1]);
    FOR(i = 2; i < FNUM; i++)
    {
        sum_mean  = add(sum_mean, w->pre_ild_sum[i]);
        sum_meanH = add(sum_meanH,w->pre_ild_sum_H[i]);
    }

    w->pos = add(w->pos,1);
    if(sub(w->pos,FNUM) >=0 )
    {
        w->pos = 0;
    }

    sum_mean  = mult(inv_FNUM,sum_mean);
    sum_meanH = mult(inv_FNUM,sum_meanH);
    /*Calculate the distance*/
    dev  = abs_s(sub(ILD_sum, sum_mean));
    devH = abs_s(sub(ILD_sumH, sum_meanH));

    flag= 0; move16(); 
    if (sub(dev, 3360) > 0) 
        flag = add(flag,1); 
    if(sub(devH, 1232)>0) 
        flag = add(flag,1); 
    flag = s_min(flag,1); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(flag); 
}

/*************************************************************************
* ild_attack_detect_shb
*
* Super higher band stereo transient detection
**************************************************************************/
Word16 ild_attack_detect_shb(Word32* L_ener, /* i: energy per sub-band of L channel */
                             Word32* R_ener, /* i: energy per sub-band of R channel */
                             Word16 q_left_en,
                             Word16 q_right_en,
                             void* ptr
                             )
{
    Word16 i;
    Word16 sum_mean;
    Word16 dev;

    Word16 flag;
    Word16 ILD;
    Word16 ILD_sum;
    Word16 tmp;
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (7 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    flag = 0; move16();

    tmp     = sub(q_left_en, q_right_en);
    ILD_sum = ild_calc_dect(L_ener[0],R_ener[0], tmp);
    ILD     = ild_calc_dect(L_ener[1],R_ener[1], tmp);
    ILD_sum = add(ILD_sum,ILD);

    w->pre_ild_sum_swb[w->pos]   = ILD_sum;  move16();

    sum_mean = add(w->pre_ild_sum_swb[0], w->pre_ild_sum_swb[1]);
    FOR(i = 2; i < FNUM; i++)
    {
        sum_mean = add(sum_mean,w->pre_ild_sum_swb[i]);
    }
    sum_mean = mult(sum_mean,inv_FNUM);

    dev = abs_s(sub(ILD_sum, sum_mean));

    if(sub(dev, 548) > 0)
    {
        flag = 1;move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(flag);
}

/*-----------------------------------------------------------------------*
*                                                                        *
*                   Quantization of stereo parameters                    *
*                                                                        *
*------------------------------------------------------------------------*/
static void searchNeighborPWQU (Word16 *idxQ0, Word16 val, Word16 valQ,
                                Word16 halfStepQ, const Word16 *indPWQU);
static Word16 searchSegPWQU_5seg(Word16 val);
static Word16 searchSegPWQU_3seg(Word16 val, const Word16 *threshPWQU);
static Word16 searchSegQ_2bits(Word16 val);
static Word16 quantUniform(Word16 param,
                     const Word16 *ptrParam, 
                     const Word16 *tabQ, 
                           Word16 *index);
static Word16 searchIdxPWQU_5seg_5bits(Word16 val);
static Word16 searchIdxPWQU_3seg_4bits(Word16 val);
static Word16 searchIdxPWQU_3seg_3bits(Word16 val);
static Word16 searchIdxQU(Word16 param, /* i: parameter value to quantize*/                
                    const Word16 *ptrParam, 
                    const Word16 *tabQ);
/*************************************************************************
* searchIdxQU: search the quantization index of uniform quantizer on 4 or 5 bits
* routine called by quantWholeWB_ITDorIPD to quantize whole wideband IPD on 4 bits (16 levels)
* quantizer in tab_phase_q4
* routine called by quantUniform to quantize individuals IPDs with 5 or 4 bits  
* parameters of uniform quantizer in ParamQuantPhase: 
* 1/stepQ, nbLevels/2-1, stepQ/2, 0, nbLevels-1
* uniform quantizer levels in tabQ (tab_phase_q5 or tab_phase_q4)
**************************************************************************/
static Word16 searchIdxQU(Word16 param, /* i: parameter value to quantize*/                
                    const Word16 *ptrParam, 
                    const Word16 *tabQ)
{
    Word16 idxQ0, valQ; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + 0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    /* parameters of uniform phase quantizers */
    idxQ0 = mult(param, *ptrParam++);  /* param*invStepQ */
    idxQ0 = add(idxQ0, *ptrParam++);  /* param*invStepQ + nbQLev/2*/
    valQ  = tabQ[idxQ0]; move16(); 

    searchNeighborPWQU(&idxQ0, param, valQ, *ptrParam, ptrParam+1);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(idxQ0);
}

/*************************************************************************
* searchSegPWQU_3seg
* routine called by searchIdxPWQU_3seg_3bits and searchIdxPWQU_3seg_4bits
* search the segment in a piece wise uniform quantizer with 3 segments
* segment thresholds in threshPWQU4 or threshPWQU3
**************************************************************************/
static Word16 searchSegPWQU_3seg(Word16 val, const Word16 *threshPWQU)
{
    Word16 iseg;
    const Word16 *ptrThreshPWQU;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    ptrThreshPWQU = threshPWQU;
    iseg = 0; move16();
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(iseg);
}

/*************************************************************************
* searchSegQ_2bits
* routine called by quantRefineILD
* search the segment in a scalar quantizer with 4 levels (2bits)
* quantizer thresholds in threshPWQU2
**************************************************************************/
static Word16 searchSegQ_2bits(Word16 val)
{
    Word16 iseg;
    const Word16 *ptrThreshPWQU;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    ptrThreshPWQU = threshPWQU2;
    iseg = 0; move16();
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(iseg);
}

/*************************************************************************
* searchSegPWQU_5seg
* routine called by searchIdxPWQU_5seg_5bits( ILDs quantization with 5 bits)  
* search the segment in a piece wise uniform quantizer with 5 segments
* segment thresholds in threshPWQU5 
**************************************************************************/
static Word16 searchSegPWQU_5seg (Word16 val)
{
    Word16 iseg;
    const Word16 *ptrThreshPWQU;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    ptrThreshPWQU = threshPWQU5;
    iseg = 0; move16();
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
    if(sub(val, *ptrThreshPWQU++) > 0) {
        iseg = add(iseg, 1);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(iseg);
}

/*************************************************************************
* quantUniform
* routine called by quantIPD: uniform quantization of IPDs with 5 or 4 bits  
* parameters of uniform quantizer in ParamQuantPhase: 
* 1/stepQ, nbLevels/2-1, stepQ/2, nbLevels-1
* uniform quantizer levels in tabQ (tab_phase_q5 or tab_phase_q4)
**************************************************************************/
Word16 quantUniform(      Word16 param, 
                    const Word16 *ptrParam, 
                    const Word16 *tabQ, 
                          Word16 *index
                    )
{
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 + 0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    *index = searchIdxQU(param, ptrParam, tabQ); move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return tabQ[*index];
}

/*************************************************************************
* searchIdxPWQU_3seg_4bits
* routine called by quantRefineILD, quantILD0, calc_quantILD_diff
* piece wise uniform quantization with 3 segments and 4 bits 
* quantizer: tab_ild_q4
**************************************************************************/
static Word16 searchIdxPWQU_3seg_4bits(Word16 val)
{
    Word16 iseg, idxQ, val2, valQ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    iseg = searchSegPWQU_3seg (val, threshPWQU4); 
    val2 = add(val, bSeg4[iseg]); 
    idxQ = mult(val2, invStepQ4[iseg]);
    idxQ = add(idxQ, ind0Seg4[iseg]);
    valQ = tab_ild_q4[idxQ];move16();
    searchNeighborPWQU(&idxQ,val, valQ, halfStepQ4[iseg], &indPWQU4[iseg]); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(idxQ);
}

/*************************************************************************
* searchIdxPWQU_3seg_3bits
* routine called by quantRefineILD and quantILD0
* piece wise uniform quantization with 3 segments on 3 bits
* quantizer: tab_ild_q3
**************************************************************************/
static Word16 searchIdxPWQU_3seg_3bits(Word16 val)
{
    Word16 idxQ, iseg;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    iseg = searchSegPWQU_3seg (val, threshPWQU3);
    idxQ = initIdxQ3[iseg]; move16();
    IF(sub(iseg,1) == 0) {
        idxQ = add(shr(val, 11), idxQ);
        searchNeighborPWQU (&idxQ, val, tab_ild_q3[idxQ], HALFSTEPQ3, indPWQU3);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(idxQ);
}

/*************************************************************************
* searchIdxPWQU_5seg_5bits
* routine called by G722_stereo_encoder_shb, quantILD0 and and calc_quantILD_abs
* piece wise uniform quantization of ILD with 5 bits
**************************************************************************/
static Word16 searchIdxPWQU_5seg_5bits(Word16 val)
{
    Word16 iseg, idxQ, val2, valQ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    iseg = searchSegPWQU_5seg (val); 
    val2 = add(val, bSeg5[iseg]); 
    idxQ = mult(val2, invStepQ5[iseg]);
    idxQ = add(idxQ, ind0Seg5[iseg]);
    valQ = tab_ild_q5[idxQ];move16();
    searchNeighborPWQU(&idxQ, val, valQ, halfStepQ5[iseg], &indPWQU5[iseg]); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (idxQ);
}

/*************************************************************************
* searchNeighborPWQU
* routine called by searchIdxPWQU_3seg_4bits,searchIdxPWQU_3seg_3bits, 
* and searchIdxPWQU_5seg_5bits
* select the nearest neighbour between two quantization levels 
**************************************************************************/
static void searchNeighborPWQU(Word16 *idxQ0, 
                               Word16 val, 
                               Word16 valQ,
                               Word16 halfStepQ, 
                               const Word16 *indPWQU
                               ) 
{
    Word16 errQL1, errQ2, idxQ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    idxQ = *idxQ0; move16();
    errQL1 = sub(val, valQ);
    IF(errQL1 < 0) 
    {
        errQ2 = add (errQL1, halfStepQ);
        if(errQ2 <= 0) idxQ = sub(idxQ, 1);
        idxQ = s_max(*indPWQU,idxQ);
    }
    ELSE 
    {
        indPWQU++;
        errQ2 = sub(errQL1, halfStepQ);
        if(errQ2 >0) idxQ = add(idxQ, 1);
        idxQ = s_min(*indPWQU,idxQ);
    }
    *idxQ0 = idxQ; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calcGain
*
* Calculate the energy correction gain
**************************************************************************/
static Word16 calcGain( Word32 L_ener, Word32 R_ener, Word32 M_ener, Word16 Qcm) 
{
    Word32 L_temp, L_temp2;
    Word16 tmp1, tmp2,tmp3, gain;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 + 2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    gain = 16384; move16();
    IF(M_ener > 0) 
    {
        L_temp2 = L_add(L_ener,R_ener);
        L_temp  = L_shr(L_temp2 ,add(Qcm, 1));
        IF(L_sub(L_temp, M_ener) >= 0) 
        {
            gain = shl(gain,1); 
            tmp1 = norm_l(M_ener);
            L_temp  = L_shl(M_ener,tmp1);
            L_temp2 = L_shl(L_temp2,sub(tmp1,add(Qcm,2)));
            IF(L_sub(L_temp2, L_temp )< 0) 
            {
                tmp2 = round_fx(L_temp);
                tmp3 = round_fx(L_temp2);
                gain = div_s(tmp3,tmp2);
            }
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(gain);
}

/*************************************************************************
* g722_stereo_encode_const
*
* G722 stereo encoder constructor
**************************************************************************/
void *g722_stereo_encode_const()
{
    g722_stereo_encode_WORK *w = NULL;

    /* Static memory allocation */
    w = (g722_stereo_encode_WORK *)malloc(sizeof(g722_stereo_encode_WORK));

    if (w != NULL)
    {
        g722_stereo_encode_reset((void *)w);
    }
    return (void *)w;
}

/*************************************************************************
* g722_stereo_encode_dest
*
* G722 stereo encoder destructor
**************************************************************************/
void g722_stereo_encode_dest( void*  p_work)   /* (i): Work space */
{
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)p_work;

    if (w != NULL)
    {
        free(w);
    }
    return;
}

/*************************************************************************
* g722_stereo_encode_reset
*
* G722 stereo encoder reset
**************************************************************************/
void  g722_stereo_encode_reset( void*  p_work)   /* (i/o): Work space */
{
    g722_stereo_encode_WORK *w=(g722_stereo_encode_WORK *)p_work;

    IF (w != NULL)
    {
        /* L channel */
        zero16(58, w->mem_input_left);
        /* R channel */
        zero16(58, w->mem_input_right);

        zero32(NB_SB,w->mem_L_ener);
        zero32(NB_SB,w->mem_R_ener);

        zero16(20,w->mem_ild_q);
        zero16(20,w->pre_q_left_en_band);
        zero16(20,w->pre_q_right_en_band);
        zero16(L_FRAME_WB,w->mem_mono);
        zero16(L_FRAME_WB,w->mem_side);
        zero16(58, w->mem_mono_ifft_s); /* mono signal */
        zero16(G722_SWB_DMX_DELAY,w->mem_left);
        zero16(G722_SWB_DMX_DELAY,w->mem_right);
        zero16(FNUM,w->pre_ild_sum_swb);
        zero16(FNUM,w->pre_ild_sum);
        zero16(FNUM,w->pre_ild_sum_H);

        w->pos              = 0; move16();
        w->SWB_ILD_mode     = 0; move16();
        w->frame_flag_wb    = 0; move16();
        w->frame_flag_swb   = 0; move16();
        w->c_flag           = 0; move16();
        w->pre_flag         = 0; move16();
        w->num              = 0; move16();
        w->mem_q_channel_en = 0; move16();
        w->mem_q_mono_en    = 0; move16();
        w->swb_frame_idx    = 0; move16();

        w->mem_l_enr[0]     = 0; move32();
        w->mem_l_enr[1]     = 0; move32();
        w->mem_r_enr[0]     = 0; move32();
        w->mem_r_enr[1]     = 0; move32();
        w->mem_m_enr[0]     = 0; move32();
        w->mem_m_enr[1]     = 0; move32();

        w->fb_ITD           = 0; move16();
        w->ic_idx           = 0; move16();
        w->ic_flag          = 0; move16();
        w->ipd_num          = 0; move16();
        w->ipd_reg_num      = 0; move16();
        w->pre_Itd          = 0; move16();
        w->pre_Ipd          = 0; move16();

        w->std_itd_pos_sm   = 1280; move16(); /* 5.0f; */
        w->std_itd_neg_sm   = 1280; move16(); /* 5.0f; */
        w->nb_idx_pos_sm    = 0; move16();
        w->nb_idx_neg_sm    = 0; move16();
        w->pre_itd_neg      = 0; move16();
        w->pre_itd_pos      = 0; move16();

        zero32(STARTBANDITD+BANDITD, w->Crxyt);
        zero32(STARTBANDITD+BANDITD, w->Cixyt);
        zero32(STARTBANDITD+BANDITD, w->Crxyt2);
        zero32(STARTBANDITD+BANDITD, w->Cixyt2);

        zero16(10, w->phase_mean_buf);
        zero16(10, w->phase_mean_buf1);

        w->phase_num = 0; move32();
        w->pos1      = 0; move16();
        w->f_num     = 0; move16();
        zero32(STARTBANDITD + BANDITD, w->energy_bin_sm);

        w->en_ratio_sm          = 0; move16();
        w->phase_mean_std_sm_ct = 0; move16();
        w->mem_energyL          = 0; move32();
        w->mem_energyR          = 0; move32();
        w->pre_ipd_mean         = 0; move16();
        w->ipd_reg_num_sm       = 0; move16();
        w->phase_mean_std_sm    = 0; move16();
        w->ipd_mean_sm          = 0; move16();
    }
    return;
}

/*************************************************************************
* downmix_swb
*
* Downmix stereo to mono of superwideband part
**************************************************************************/
void downmix_swb(Word16*    input_left,  /* i: input L channel*/
                 Word16*    input_right, /* i: input R channel*/
                 Word16*    mono,        /* o: mono signal */
                 Word16*    side,
                 void*      ptr
                 )
{
    Word16 i;

    Word16 tmp_l,tmp_r;
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 + 1 * SIZE_Ptr), "dummy");
#endif

    FOR(i = 0; i < L_FRAME_WB; i ++)
    {
        tmp_l = shr(w->mem_left[i],1);
        tmp_r = shr(w->mem_right[i],1);
        mono[i] = add(tmp_l, tmp_r); move16();
        side[i] = sub(tmp_l, tmp_r); move16();
    }
    mov16(G722_SWB_DMX_DELAY, input_left, w->mem_left);
    mov16(G722_SWB_DMX_DELAY, input_right, w->mem_right);

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

static Word16 getIPD(Word16 swb_flag, Word16 *L_real, Word16 *L_imag, 
                     Word16 *R_real, Word16 *R_imag, Word16 *IPD, 
                     Word32 *eLeftS, Word32 *eRightS);
static Word16 computeMonoDownmix(Word16 swb_flag, Word16 q_left, Word16 q_right, 
                                 Word16 *mem_ild_q, Word16 *IPD, Word16 fb_IPD, 
                                 Word16 *mono_real, Word16 *mono_imag, 
                                 Word32 *eLeftS, Word32 *eRightS, 
                                 Word16 *L_real, Word16 *L_imag);
static void calcPhaseDownmix(Word16 nbCoef, Word16 mem_ild_dq, Word16 *IPD, 
                             Word16 fb_IPD, Word16 *L_real, Word16 *L_imag,
                             Word16 iTmpHi, Word32 *L_ptr, Word32 *L_ptr2, 
                             Word16 *mono_real, Word16 *mono_imag); 
static void quantIPD(Word16 mode, Word16 swb_flag, Word16 SWB_ILD_mode, 
                     Word16 *IPD, Word16 *idx);

/*************************************************************************
* downmix
*
* Downmix stereo to mono in frequency domain
**************************************************************************/
void downmix(Word16 *input_left,  /* i: input L channel*/
             Word16 *input_right, /* i: input R channel*/
             Word16 *mono,    /* o: mono signal */
             void   *ptr,
             Word16 *bpt_stereo,
             Word16 mode,
             Word16 *frame_idx,
             Word16 Ops
             )
{
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr;
    Word16 i;
    Word16 L_real[NFFT + 2];
    Word16 R_real[NFFT + 2];
    Word16 *L_imag = &L_real[81],*R_imag = &R_real[81];
    Word16 q_left;
    Word16 q_right;
    /* downmix in FFT domain */

    Word16 mono_real[NFFT + 2];            
    Word16 *mono_imag = &mono_real[NFFT/2 + 1]; 
    Word16 IPD[NFFT/2 + 1];
    Word16 q_mono; 
    Word32 eLeftS[81], eRightS[81] ;
    Word16 swb_flag;
    Word16 nbCoef;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((6 + 3 * (NFFT + 2) + (NFFT/2 + 1)) * SIZE_Word16 +  (2 * 81) * SIZE_Word32 +
        4 * SIZE_Ptr), "dummy");
#endif

    swb_flag = (sub(Ops, 32000)==0);
#ifdef WMOPS_IDX
    setCounter(Id_dmx);
#endif
    windowStereo(input_left,  w->mem_input_left,  L_real);
    windowStereo(input_right, w->mem_input_right, R_real);

#ifdef WMOPS_IDX
    setCounter(Id_fft);
#endif
    fixDoRFFTx(L_real, &q_left);
    fixDoRFFTx(R_real, &q_right);

#ifdef WMOPS_IDX
    setCounter(Id_dmx); 
#endif
    nbCoef = getIPD(swb_flag, L_real, L_imag, R_real, R_imag, IPD, eLeftS, eRightS);
#ifdef WMOPS_IDX
    setCounter(Id_itd); 
#endif
    /*extract full band ITD,IPD and IC */
    get_interchannel_difference(w, L_real, L_imag, q_left, R_real, 
                                R_imag, q_right, &w->ic_idx, &w->ic_flag);

#ifdef WMOPS_IDX
    setCounter(Id_st_enc); 
#endif
    IF(sub(mode, MODE_R1ws) !=0) 
    {
        quantIPD(mode, swb_flag, w->SWB_ILD_mode,  IPD, w->idx);
    }
    /*mainloop for ILD estimation, quantization and stereo bitstream writing*/
    g722_stereo_encode(L_real, L_imag, R_real, R_imag, q_left, q_right, bpt_stereo, w, frame_idx, mode ,Ops);

#ifdef WMOPS_IDX
    setCounter(Id_dmx); 
#endif
    q_mono = computeMonoDownmix(swb_flag, q_left, q_right, w->mem_ild_q, IPD, w->fb_IPD, 
                                mono_real, mono_imag, eLeftS, eRightS, L_real, L_imag);


#ifdef WMOPS_IDX
    setCounter(Id_ifft);
#endif
    fixDoRiFFTx(mono_real, &q_mono);

#ifdef WMOPS_IDX
    setCounter(Id_dmx);
#endif
    /* overlap and add downmix mono */
    OLA( &mono_real[11], w->mem_mono_ifft_s, mono);

    /* update memory */
    FOR(i=0; i<58; i++)
    {
        w->mem_input_left[i]  = input_left[i + 22];  move16();
        w->mem_input_right[i] = input_right[i + 22]; move16();
        w->mem_mono_ifft_s[i] = mult(mono_real[i + NFFT/2 + 11], win_D[58 - i - 1]); move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* quantIPD
*
* Quantize IPD
**************************************************************************/
static void quantIPD(Word16 mode, Word16 swb_flag, Word16 SWB_ILD_mode,  
                     Word16 *IPD, Word16 *idx)
{
    Word16 nbBand, j;
    Word16 *ptrIPD, *ptrIdx;
    
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2) * SIZE_Word16 +  (0) * SIZE_Word32 + 2 * SIZE_Ptr, "dummy");
#endif
    nbBand = sub(sub(IPD_SYN_WB_END_WB+1, IPD_SYN_START), shl(swb_flag,1) );
    ptrIPD = IPD + IPD_SYN_START;
    ptrIdx = idx + IPD_SYN_START;
  
    FOR(j=0; j<nbBand; j++)
    {
        *ptrIPD = quantUniform(*ptrIPD, paramQuantPhase,tab_phase_q5, ptrIdx); move16();
        ptrIPD++; ptrIdx++;
    }

    IF(swb_flag)
    {
        /*if swb_ILD_mode =0  IPD[8] quantized on 4 bits  else swb_ILD_mode =1 IPD[8] quantized on 5 bits */
        IF(SWB_ILD_mode == 0)
        {
            *ptrIPD = quantUniform(*ptrIPD, paramQuantPhase+5,tab_phase_q4, ptrIdx); move16();
        }
        ELSE
        {
            *ptrIPD = quantUniform(*ptrIPD, paramQuantPhase,tab_phase_q5, ptrIdx); move16();
        }
        IF(sub(mode, MODE_R5ss) == 0)
        { /* for R5ss only : quantize 16 IPDs - bins 9 to 24 - each quantized with 5 bits*/
            nbBand = sub(IPD_SYN_SWB_END_SWB+1, IPD_SYN_WB_END_SWB + 1);
            ptrIPD++; ptrIdx++;

            FOR(j=0; j<nbBand; j++)
            {
                *ptrIPD = quantUniform(*ptrIPD, paramQuantPhase,tab_phase_q5, ptrIdx); move16();
                ptrIPD++; ptrIdx++;
            }
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* getIPD
*
* Compute IPD per frequency bin
**************************************************************************/
static Word16 getIPD(Word16 swb_flag, Word16 *L_real, Word16 *L_imag, 
                     Word16 *R_real, Word16 *R_imag, Word16 *IPD, 
                     Word32 *eLeftS, Word32 *eRightS)
{
    Word16 j, nbCoef;
    Word32 L_tmp, L_tmp2;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2) * SIZE_Word16 +  ( 2) * SIZE_Word32 +0 * SIZE_Ptr, "dummy");
#endif
    nbCoef = 81; move16();
    IF(swb_flag == 0)
    {
        nbCoef = 71; move16();
        /* zeroing the interval [7000, 8000 Hz] */
        FOR(j=71; j<(NFFT/2+1); j++)
        {
            L_real[j]  = 0; move16();
            L_imag[j]  = 0; move16();
            R_real[j]  = 0; move16();
            R_imag[j]  = 0; move16();
            IPD[j]     = 0; move16();
            eLeftS[j]  = 0; move32();
            eRightS[j] = 0; move32();
        }
    }

    /*  Get IPD of current frame */
    FOR(j= 0;j< nbCoef ;j++)
    {
        L_tmp      = L_mult(L_real[j], L_real[j]);
        L_tmp      = L_mac(L_tmp, L_imag[j], L_imag[j]); // 2*q_left +1 
        SqrtI31(L_tmp, &eLeftS[j]) ;       // (31-( 31- q )/2). =  16 + q_left

        L_tmp      = L_mult(R_real[j], R_real[j]);
        L_tmp      = L_mac(L_tmp, R_imag[j], R_imag[j]); // 2*q_right +1 
        SqrtI31(L_tmp, &eRightS[j]);      // (31-( 31- q )/2). =  16 + q_right

        L_tmp      = L_mac(1, L_real[j], R_real[j]);       
        L_tmp2     = L_mac(L_tmp, L_imag[j], R_imag[j]);

        L_tmp      = L_mult(L_imag[j], R_real[j]);
        L_tmp      = L_msu(L_tmp, L_real[j], R_imag[j]); 
        IPD[j] = arctan2_fix32(L_tmp, L_tmp2); move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    move16();
    return (nbCoef);
}

/*************************************************************************
* computeMonoDownmix
*
* Compute mono downmix
**************************************************************************/
static Word16 computeMonoDownmix(Word16 swb_flag, Word16 q_left, Word16 q_right, Word16 *mem_ild_q,
                                 Word16 *IPD, Word16 fb_IPD, Word16 *mono_real, Word16 *mono_imag, 
                                 Word32 *eLeftS, Word32 *eRightS, Word16 *L_real, Word16 *L_imag)
{
    Word16 iTmpHi, q_mono, nbCoef, mem_ild_dq, i, j;
    Word32 *L_ptr, *L_ptr2;
    Word16 *ptrIPD, *ptrReal, *ptrImag, *ptr_mono_real, *ptr_mono_imag;
    const Word16 *ptr0 = c_table10 + 80;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (6) * SIZE_Word16 +  ( 2) * SIZE_Word32 +6 * SIZE_Ptr, "dummy");
#endif

    iTmpHi = sub(q_left, q_right);
    L_ptr  = eLeftS;
    L_ptr2 = eRightS;
    IF(iTmpHi < 0)
    {
        L_ptr  = eRightS;
        L_ptr2 = eLeftS;
    }
    q_mono = s_min(q_left, q_right);
    q_mono = sub(q_mono, 16);
    iTmpHi = abs_s(iTmpHi);
    iTmpHi = add(iTmpHi,1);

    ptrIPD  = IPD;
    ptrReal = L_real;
    ptrImag = L_imag;
    ptr_mono_real = mono_real;
    ptr_mono_imag = mono_imag;

    FOR (i = 0; i < NB_SB-1; i++)
    {
        j = shr(mem_ild_q[i],9);
        mem_ild_dq = ptr0[j]; move16();
        nbCoef = nbCoefBand[i]; move16();
        calcPhaseDownmix(nbCoef, mem_ild_dq, ptrIPD, fb_IPD, ptrReal, ptrImag,
                         iTmpHi, L_ptr, L_ptr2, ptr_mono_real, ptr_mono_imag) ;
        ptrIPD  += nbCoef;
        ptrReal += nbCoef;
        ptrImag += nbCoef;
        L_ptr   += nbCoef;
        L_ptr2  += nbCoef;
        ptr_mono_real += nbCoef;
        ptr_mono_imag += nbCoef;
    }

    nbCoef = nbCoefBand[NB_SB-1];
    j = shr(mem_ild_q[NB_SB-1],9);
    mem_ild_dq = ptr0[j]; move16();
    IF (swb_flag == 0)
    {
        /* zeroing the interval [7000, 8000 Hz] */
        FOR(j=71; j<(NFFT/2+1); j++)
        {
            mono_real[j] = 0; move16();
            mono_imag[j] = 0; move16();
        }
        nbCoef = sub(nbCoef,10);
    }
    calcPhaseDownmix(nbCoef, mem_ild_dq, ptrIPD, fb_IPD, ptrReal, ptrImag,
                     iTmpHi, L_ptr, L_ptr2, ptr_mono_real, ptr_mono_imag) ;
    
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    move16();
    return (q_mono);
}

/*************************************************************************
* calcPhaseDownmix
*
* Compute phase of the mono downmix
**************************************************************************/
static void calcPhaseDownmix(Word16 nbCoef, Word16 mem_ild_dq, Word16 *IPD, 
                             Word16 fb_IPD, Word16 *L_real, Word16 *L_imag,
                             Word16 iTmpHi, Word32 *L_ptr, Word32 *L_ptr2, 
                             Word16 *mono_real, Word16 *mono_imag) 
{
    Word16 j, tmp16;
    Word16 iCPhase, Dmx_arg, L_arg, iPhaseCos, iTmpPhase, iPhaseSin;
    Word32 L_tmp, mono_mag;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (12) * SIZE_Word16 +  ( 2) * SIZE_Word32 +2 * SIZE_Ptr, "dummy");
#endif

    FOR(j = 0; j < nbCoef; j++)
    {
        /*calculate the phase of downmix signal*/
        tmp16   = sub( IPD[j], fb_IPD );
        IPD[j]  = Round_Phase(tmp16); move16();
        iCPhase = mult(mem_ild_dq,IPD[j]);
        L_arg = arctan2_fix32( L_deposit_l(L_imag[j]), L_deposit_l(L_real[j])); //Q12

        Dmx_arg = sub(L_arg, iCPhase);
        iPhaseCos =  spx_cos(Dmx_arg);// Q15

        iTmpPhase =  sub( PID2_FQ12 , Dmx_arg ) ;
        iPhaseSin =  spx_cos(iTmpPhase)  ; // Q15

        /*calculate the amplitude of downmix signal*/
        mono_mag = L_add( L_shr( *L_ptr++,iTmpHi) ,L_shr( *L_ptr2++,1 ));

        /*calculate the real and imag of downmix signal*/
        tmp16 = round_fx(mono_mag); 
        L_tmp = L_mult(iPhaseCos ,  tmp16 ); 
        mono_real[j] = round_fx(L_tmp); move16(); 
        L_tmp = L_mult(iPhaseSin ,  tmp16 ); 
        mono_imag[j] = round_fx(L_tmp); move16(); ; 
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

static void calcEnerBandShb(Word16 *mono_mdct, Word16 *side_mdct, 
                            Word16 norm_mono, Word16 norm_side, 
                            Word32 *enerM, Word32 *enerL,  Word32 *enerR,
                            Word16 *q_mono_en, Word16 *q_channel_en);
static Word32 calcEnerOneBandSwb0(Word16 n, Word16 *ptr);
static Word32 calcEnerOneBandSwb1(Word16 n, Word16 *ptr, Word16 nbShr);
static Word32 calcXYOneBandSwb1(Word16 n, Word16 *ptr0, Word16 *ptr1, Word16 nbShr);
static Word32 calcXYOneBandSwb0(Word16 n, Word16 *ptr0, Word16 *ptr1);
static void calcEnerLRSwb(Word16 nbShM, Word16 nbShS, Word16 nbShMS,
                          Word32 enerM, Word32 enerS, Word32 enerMS, 
                          Word32 *enerL, Word32 *enerR, Word16 *q_channel_en); 
static Word16 setSameQval(Word16 diffQ, Word16 *nbSh, Word32 *ener);
static Word16 setSameQval2(Word16 diffQ, Word16 *nbSh, Word32 *enerL, Word32 *enerR );

/*************************************************************************
* calcEnerBandShb
* routine called by G722_stereo_encoder_shb
* Calculation of energy in the 2 SHB sub bands: mono, left and right channels
**************************************************************************/
static void calcEnerBandShb(Word16 *mono_mdct, Word16 *side_mdct, 
                            Word16 norm_mono, Word16 norm_side, 
                            Word32 *enerM, Word32 *enerL, Word32 *enerR, 
                            Word16 *q_mono_en, Word16 *q_channel_en)
{
    Word32 *ptrE;
    Word16 q_mono_en_loc[2], q_channel_en_loc[2];
    Word32 ener_loc[4], *enerS, *enerMS;
    Word16 nbSh[4], *nbShM, *nbShS, *nbShMS;
    Word16 *ptr, *ptr_sh, tmp, normM, normS, normMS;
    Word16 norm_mono2, norm_side2, norm_ms;
    Word16 *ptrm, *ptrs;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((4 + 2 * 2 + 4 * 1) * SIZE_Word16 +  (4 * 1) * SIZE_Word32 + 10 * SIZE_Ptr), "dummy");
#endif

    enerS  = ener_loc;
    enerMS = ener_loc+2;
    nbShM  = q_mono_en_loc;
    nbShS  = nbSh;
    nbShMS = nbSh+2;
    normM  = Exp16Array(60, mono_mdct);
    norm_mono2 = shl(norm_mono, 1);
    norm_side2 = shl(norm_side, 1);
    norm_ms    = add(norm_mono, norm_side);

    /* compute ener mono */
    ptr_sh = nbShM;
    ptr    = mono_mdct;
    ptrE   = enerM;
    tmp    = sub(2,normM);
    IF(tmp<=0)
    {
        *ptrE++ = calcEnerOneBandSwb0(20, ptr); move32();
        *ptr_sh++ = norm_mono2; move16();
    }
    ELSE
    {
        tmp = shl(tmp,1);
        *ptrE++ = calcEnerOneBandSwb1(20, ptr,tmp); move32();
        *ptr_sh++ = sub(norm_mono2,tmp); move16();
    }
    ptr += 20;
    tmp = sub(3,normM);
    IF(tmp<=0)
    {
        *ptrE++ = calcEnerOneBandSwb0(40, ptr); move32();
        *ptr_sh++ = norm_mono2; move16();
    }
    ELSE
    {
        tmp = sub(shl(tmp,1),1);
        *ptrE++ = calcEnerOneBandSwb1(40, ptr,tmp); move32();
        *ptr_sh++ = sub(norm_mono2, tmp); move16();
    }
    /* compute ener side */
    normS  = Exp16Array(60, side_mdct);
    ptr_sh = nbSh;
    ptr    = side_mdct;
    ptrE   = ener_loc;
    tmp    = sub(2,normS);
    IF(tmp<=0)
    {
        *ptrE++ = calcEnerOneBandSwb0(20, ptr); move32();
        *ptr_sh++ = norm_side2; move16();
    }
    ELSE
    {
        tmp = shl(tmp,1);
        *ptrE++ = calcEnerOneBandSwb1(20, ptr,tmp); move32();
        *ptr_sh++ = sub(norm_side2,tmp); move16();
    }
    ptr += 20;
    tmp = sub(3,normS);
    IF(tmp<=0)
    {
        *ptrE++ = calcEnerOneBandSwb0(40, ptr); move32();
        *ptr_sh++ = norm_side2; move16();
    }
    ELSE
    {
        tmp = sub(shl(tmp,1),1);
        *ptrE++ = calcEnerOneBandSwb1(40, ptr,tmp); move32();
        *ptr_sh++ = sub(norm_side2,tmp); move16();
    }

    /* compute M*S */
    ptrm   = mono_mdct;
    ptrs   = side_mdct;
    ptrE   = enerMS;
    normMS = add(normM, normS);

    tmp = sub(4,normMS);
    IF(tmp<=0)
    {
        *ptrE++ = calcXYOneBandSwb0(20, ptrm, ptrs); move32();
        *ptr_sh++ = norm_ms; move16();
    }
    ELSE
    {
        *ptrE++ = calcXYOneBandSwb1(20, ptrm, ptrs, tmp); move32();
        *ptr_sh++ = sub(norm_ms, tmp); move16();
    }
    ptrm += 20;
    ptrs += 20;

    tmp = sub(5,normMS);
    IF(tmp<=0)
    {
        *ptrE++ = calcXYOneBandSwb0(40, ptrm, ptrs); move32();
        *ptr_sh++ = norm_ms; move16();
    }
    ELSE
    {
        *ptrE++ = calcXYOneBandSwb1(40, ptrm, ptrs, tmp); move32();
        *ptr_sh++ = sub(norm_ms,tmp); move16();
    }

    /* compute ener left and right and and update q_en shr */
    calcEnerLRSwb(nbShM[0], nbShS[0],nbShMS[0],enerM[0],enerS[0], enerMS[0],
                  &enerL[0], &enerR[0],&q_channel_en_loc[0]);

    calcEnerLRSwb(nbShM[1], nbShS[1],nbShMS[1],enerM[1],enerS[1], enerMS[1],
                  &enerL[1], &enerR[1],&q_channel_en_loc[1]);

    /*set QVal for both bands */
    tmp = sub(q_mono_en_loc[0], q_mono_en_loc[1]);
    *q_mono_en = q_mono_en_loc[0];
    IF(tmp > 0)
    {
        *q_mono_en = setSameQval(tmp, nbShM, enerM);
    }
    tmp = sub(q_channel_en_loc[0], q_channel_en_loc[1]);
    *q_channel_en = q_channel_en_loc[0];
    IF(tmp > 0)
    {
        *q_channel_en = setSameQval2(tmp, q_channel_en_loc, enerL, enerR );
    }
    *q_channel_en = sub(*q_channel_en, 2); 

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* setSameQval
* routine called by calcEnerBandShb
* set the two SHB sub band energies to the same Q value
**************************************************************************/
static Word16 setSameQval(Word16 diffQ, Word16 *nbSh, Word32 *ener)
{
    Word16 qVal;
    Word16 norm1, tmp;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    qVal = nbSh[0]; move16();
    IF(ener[1] != 0)
    {
        norm1 = norm_l(ener[1]);
        tmp = sub(norm1,diffQ);
        IF(tmp >=0) 
        {
            ener[1] = L_shl(ener[1],diffQ);
        }
        ELSE
        {
            ener[0] = L_shl(ener[0],tmp);
            ener[1] = L_shl(ener[1],norm1);
            qVal = add(nbSh[0], tmp);
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(qVal);
}

/*************************************************************************
* setSameQval2
* routine called by calcEnerBandShb
* set the SHB sub bands  of the left and right channles to the same Q value
**************************************************************************/
static Word16 setSameQval2(Word16 diffQ, Word16 *nbSh, Word32 *enerL, Word32 *enerR )
{
    Word16 qVal;
    Word16 norm1, tmp;
    Word32 Lmax;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    Lmax = L_max(enerL[1], enerR[1]);

    qVal = nbSh[0]; move16();
    IF(Lmax !=0)
    {
        norm1 = norm_l(Lmax);
        tmp   = sub(norm1,diffQ);
        IF(tmp >= 0) 
        {
            enerL[1] = L_shl(enerL[1],diffQ);
            enerR[1] = L_shl(enerR[1],diffQ);
        }
        ELSE
        {
            enerL[0] = L_shl(enerL[0],tmp);
            enerL[1] = L_shl(enerL[1],norm1);
            enerR[0] = L_shl(enerR[0],tmp);
            enerR[1] = L_shl(enerR[1],norm1);
            qVal = add(nbSh[0], tmp);
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(qVal);
}

/*************************************************************************
* calcEnerLRSwb
* routine called by calcEnerBandShb
* compute the SHB sub band energies of left and right channels
**************************************************************************/
static void calcEnerLRSwb(Word16 nbShM, Word16 nbShS, Word16 nbShMS,
                          Word32 enerM, Word32 enerS, Word32 enerMS, 
                          Word32 *enerL, Word32 *enerR, Word16 *q_channel_en) 
{
    Word16 tmp, tmp1, tmp2;
    Word32 enerM_S, Ltemp, Ltemp2;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  3 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    tmp = sub(nbShM, nbShS);
    IF(tmp >= 0) 
    {
        tmp1 = add(tmp,1); 
        enerM_S = L_add(L_shr(enerM,tmp1), L_shr(enerS,1));

    }
    ELSE
    {
        tmp  = negate(tmp);
        tmp1 = add(tmp,1); 
        enerM_S = L_add(L_shr(enerM,1), L_shr(enerS,tmp1));
    }
    tmp1 = s_min(nbShM, nbShS);
    tmp2 = sub(tmp1, nbShMS);
    IF(tmp2 >= 0) 
    {
        Ltemp  = L_shr(enerM_S, add(tmp2,1));
        Ltemp2 = L_shr(enerMS,1);
    }
    ELSE
    {
        tmp2   = negate(tmp2);
        Ltemp  = L_shr(enerM_S,1);
        Ltemp2 = L_shr(enerMS, add(tmp2,1));
    }
    *enerL = L_add(Ltemp, Ltemp2);
    *enerR = L_sub(Ltemp, Ltemp2);
    *q_channel_en = s_min(tmp1, nbShMS);

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calcXYOneBandSwb0
* routine called by calcEnerBandShb
* compute  mono and side signals cross-correlation SUM mono(i)*side(i)
* without saturation (no need to shift right )
**************************************************************************/
static Word32 calcXYOneBandSwb0(Word16 n, Word16 *ptr0, Word16 *ptr1)
{
    Word32 L_temp;
    Word16 j;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp = L_mult0(*ptr0++, *ptr1++);
    FOR(j=1; j<n; j++) 
    {
        L_temp = L_mac0(L_temp, *ptr0++, *ptr1++);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp);
}

/*************************************************************************
* calcXYOneBandSwb1
* routine called by calcEnerBandShb
* compute  mono and side signals cross-correlation SUM mono(i)*side(i)
* with right shift to avoid saturation
**************************************************************************/
static Word32 calcXYOneBandSwb1(Word16 n, Word16 *ptr0, Word16 *ptr1, Word16 nbShr)
{
    Word32 L_temp, L_temp2;
    Word16 j;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp  = L_mult0(*ptr0++, *ptr1++);
    L_temp2 = L_shr(L_temp, nbShr); 
    FOR(j=1; j<n; j++) 
    {
        L_temp  = L_mult0(*ptr0++, *ptr1++);
        L_temp2 = L_add(L_temp2, L_shr(L_temp, nbShr)); 
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp2);
}

/*************************************************************************
* calcEnerOneBandSwb0
* routine called by calcEnerBandShb
* compute  SHB sub band energies of  mono and side signals 
* without saturation (no need to shift right )
**************************************************************************/
static Word32 calcEnerOneBandSwb0(Word16 n, Word16 *ptr)
{
    Word32 L_temp;
    Word16 j;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp = L_deposit_l(1);
    FOR(j=0; j<n; j++) 
    {
        L_temp = L_mac0(L_temp, *ptr, *ptr);
        ptr++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp);
}

/*************************************************************************
* calcEnerOneBandSwb1
* routine called by calcEnerBandShb
* compute  SHB sub band energies of  mono and side signals 
* with right shift to avoid saturation
**************************************************************************/
static Word32 calcEnerOneBandSwb1(Word16 n, Word16 *ptr, Word16 nbShr)
{
    Word32 L_temp, L_temp2;
    Word16 j;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp2 = L_deposit_l(1);
    FOR(j=0; j<n; j++) 
    {
        L_temp  = L_mult0( *ptr ,*ptr);
        L_temp2 = L_add(L_temp2, L_shr(L_temp, nbShr)); 
        ptr++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp2);
}

static Word16 smoothEnerSHB_LR(Word32 *L_ener, Word32 *R_ener, Word16 nbShCur,
                               Word32 *mem_L_ener, Word32 *mem_R_ener, Word16 nbShPre);
static Word16 smoothEnerSHB_M(Word32 *M_ener, Word16 nbShCur, Word32 *mem_M_ener, Word16 nbShPre);
/*************************************************************************
* G722_stereo_encoder_shb
*
* G722 super higher band stereo encoder
**************************************************************************/
void G722_stereo_encoder_shb(Word16* mono_in,
                             Word16* side_in,
                             void*   ptr,
                             Word16* bpt_stereo_swb,
                             Word16  mode,
                             Word16  *gain
                             )
{
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr;
    Word16 mono_mdct[L_FRAME_WB],side_mdct[L_FRAME_WB];
    Word16 i;
    Word16 *swb_pt = bpt_stereo_swb;
    Word16 mono[L_FRAME_WB];
    Word16 norm_mono,norm_side;
    Word16 q_channel_en,q_mono_en;
    Word16 idx;
    Word16 tmp,diffQ;
    Word16 nbShCur, nbShPre;
    Word32 L_ener[SWB_BN],R_ener[SWB_BN]; 
    Word32 M_ener[SWB_BN];

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((10 + 3 * L_FRAME_WB) * SIZE_Word16 +  (3 * SWB_BN) * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif
    FOR(i=0; i<L_FRAME_WB; i+=2)
    {
        mono[i]    = negate(mono_in[i]); move16();
        mono[i+1]  = mono_in[i+1];       move16();
        side_in[i] = negate(side_in[i]); move16();
    }
    /*mdct of mono and side signal*/
    bwe_mdct( w->mem_mono, mono, mono_mdct, &norm_mono );
    bwe_mdct( w->mem_side, side_in, side_mdct, &norm_side );

    calcEnerBandShb(mono_mdct, side_mdct, norm_mono, norm_side, M_ener, 
                    L_ener, R_ener, &q_mono_en, &q_channel_en);

    /* ILD attack detection for SHB*/
    w->SWB_ILD_mode = ild_attack_detect_shb(L_ener, R_ener, q_channel_en, q_channel_en, w);

    IF(w->SWB_ILD_mode == 1) /*1 frames mode*/
    {
        swb_pt += 74; /*make room for the wb stereo bits*/
        w->swb_frame_idx = 0;move16();
        L_ener[0] = L_add(L_shr(L_ener[0],1),L_shr(L_ener[1],1)); move32();
        R_ener[0] = L_add(L_shr(R_ener[0],1),L_shr(R_ener[1],1)); move32();
        M_ener[0] = L_add(L_shr(M_ener[0],1),L_shr(M_ener[1],1)); move32();
        
        /*calculate and quantize ILD in SHB with 5bits*/
        tmp = ild_calculation(L_ener[0],R_ener[0], 0, 0);
        idx = searchIdxPWQU_5seg_5bits(tmp); move16();
        write_index5(swb_pt, idx);

        /* update memory last SHB band*/
        w->mem_l_enr[1] = L_ener[0]; move32();
        w->mem_r_enr[1] = R_ener[0]; move32();
        w->mem_m_enr[1] = M_ener[0]; move32();
    }
    ELSE /*2 frames mode*/
    {
        swb_pt += 73; /*make room for the wb stereo bits*/

        IF(w->frame_flag_swb == 0)
        {
            w->mem_q_channel_en = q_channel_en; move16();
            w->mem_q_mono_en    = q_mono_en;    move16();
            w->frame_flag_swb   = 1;            move16();
        }

        /*smooth the energy between two consecutive frames*/
        diffQ = sub(w->mem_q_channel_en, q_channel_en);
        nbShCur = s_max(1, sub(1,diffQ));
        nbShPre = s_max(1, add(1,diffQ));
        tmp = smoothEnerSHB_LR(L_ener, R_ener, nbShCur, w->mem_l_enr, w->mem_r_enr, nbShPre);
        q_channel_en = add(q_channel_en,tmp);
        
        diffQ = sub(w->mem_q_mono_en, q_mono_en);
        nbShCur = s_max(1, sub(1,diffQ));
        nbShPre = s_max(1, add(1,diffQ));
        tmp = smoothEnerSHB_M(M_ener, nbShCur, w->mem_m_enr, nbShPre);
        q_mono_en = add(q_mono_en,tmp);

        /* update memory last SHB band*/
        w->mem_l_enr[1] = L_ener[1]; move32();
        w->mem_r_enr[1] = R_ener[1]; move32();
        w->mem_m_enr[1] = M_ener[1]; move32();

        write_index1(swb_pt, w->swb_frame_idx);
        swb_pt += 1;
        /*calculate and quantize the ILD in SHB with 5 bits*/
        tmp = ild_calculation(L_ener[w->swb_frame_idx],R_ener[w->swb_frame_idx], 0, 0);
        idx = searchIdxPWQU_5seg_5bits( tmp); move16();
        write_index5(swb_pt, idx);
        w->swb_frame_idx = s_and(add(w->swb_frame_idx,1) ,1);
    }

    /*calculate the energy correction gain*/
    tmp = sub(q_channel_en, q_mono_en);
    gain[0] = calcGain(L_ener[0], R_ener[0], M_ener[0], tmp); 
    gain[1] = calcGain(L_ener[1], R_ener[1], M_ener[1], tmp);

    /* update memory 1st SHB band and Q values mono and channel */
    w->mem_l_enr[0] = L_ener[0]; move32();
    w->mem_r_enr[0] = R_ener[0]; move32();
    w->mem_m_enr[0] = M_ener[0]; move32();
    w->mem_q_mono_en    = q_mono_en;    move16();
    w->mem_q_channel_en = q_channel_en; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
}

/*************************************************************************
* smoothEnerSHB_M
* routine called by G722_stereo_encoder_shb
* smooth energies of SHB sub-bands of mono signals with previous frame 
**************************************************************************/
static Word16 smoothEnerSHB_M(Word32 *M_ener, Word16 nbShCur, Word32 *mem_M_ener, Word16 nbShPre)
{
    Word32 LtmpM0, LtmpM1;
    Word16 nbShE;
    Word32 enerMax;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  3 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    LtmpM0 = L_add(L_shr(mem_M_ener[0],nbShPre),L_shr(M_ener[0],nbShCur)); 
    LtmpM1 = L_add(L_shr(mem_M_ener[1],nbShPre),L_shr(M_ener[1],nbShCur));
    enerMax = L_max(LtmpM0, LtmpM1); 
    nbShE   = norm_l(enerMax);
    M_ener[0] = L_shl(LtmpM0,nbShE);move32();
    M_ener[1] = L_shl(LtmpM1,nbShE);move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    move16();
    return (nbShE);
}

/*************************************************************************
* smoothEnerSHB_LR
* routine called by G722_stereo_encoder_shb
* smooth energies of SHB sub-bands of left and right channels with previous frame 
**************************************************************************/
static Word16 smoothEnerSHB_LR(Word32 *L_ener, Word32 *R_ener, Word16 nbShCur,
                               Word32 *mem_L_ener, Word32 *mem_R_ener, Word16 nbShPre)
{
    Word32 LtmpL0, LtmpR0, LtmpL1, LtmpR1;
    Word32 enerMax;
    Word16 nbShE;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  5 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    LtmpL0 = L_add(L_shr(mem_L_ener[0],nbShPre),L_shr(L_ener[0],nbShCur)); 
    LtmpR0 = L_add(L_shr(mem_R_ener[0],nbShPre),L_shr(R_ener[0],nbShCur)); 

    LtmpL1 = L_add(L_shr(mem_L_ener[1],nbShPre),L_shr(L_ener[1],nbShCur));
    LtmpR1 = L_add(L_shr(mem_R_ener[1],nbShPre),L_shr(R_ener[1],nbShCur));

    enerMax = L_max(LtmpL0, LtmpL1); 
    enerMax = L_max(enerMax, LtmpR0);
    enerMax = L_max(enerMax, LtmpR1);
    nbShE   = norm_l(enerMax);

    L_ener[0] = L_shl(LtmpL0,nbShE);move32();
    L_ener[1] = L_shl(LtmpL1,nbShE);move32();
    R_ener[0] = L_shl(LtmpR0,nbShE);move32();
    R_ener[1] = L_shl(LtmpR1,nbShE);move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    move16();
    return (nbShE);
}

/*************************************************************************
* calcEnerOneBand0
* routine calleb by calcEnerBand
* compute one WB sub-band energy (of left or right channel)
* without saturation (no need to shift right )
**************************************************************************/
static Word32 calcEnerOneBand0(Word16 nb, Word16 *ptr_r, Word16 *ptr_i)
{
    Word32 L_temp;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp = L_deposit_l(1);
    L_temp = L_mac0(L_temp, *ptr_r, *ptr_r);
    L_temp = L_mac0(L_temp, *ptr_i, *ptr_i);
    ptr_r++; ptr_i++;
    FOR(i = 1; i < nb; i++)
    {
        L_temp = L_mac0(L_temp, *ptr_r, *ptr_r);
        L_temp = L_mac0(L_temp, *ptr_i, *ptr_i);
        ptr_r++; ptr_i++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (L_temp);
}

/*************************************************************************
* calcEnerOneBand1
* routine called calcEnerBand
* with right shift to avoid saturation
**************************************************************************/
static Word32 calcEnerOneBand1(Word16 nb, Word16 *ptr_r, Word16 *ptr_i, Word16 nbSh) {
    Word32 L_temp, L_temp2;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp2 = L_deposit_l(1);
    L_temp  = L_mult0(*ptr_r, *ptr_r);
    L_temp  = L_mac0(L_temp, *ptr_i, *ptr_i);
    L_temp2 = L_add(L_temp2, L_shr(L_temp, nbSh));
    ptr_r++; ptr_i++;
    FOR(i = 1; i < nb; i++)
    {
        L_temp  = L_mult0(*ptr_r, *ptr_r);
        L_temp  = L_mac0(L_temp, *ptr_i, *ptr_i);
        L_temp2 = L_add(L_temp2, L_shr(L_temp, nbSh));
        ptr_r++; ptr_i++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (L_temp2);
}

/*************************************************************************
* calcEnerBand
* routine called by g722_stereo_encode
* compute WB sub-band energies of left and right channels
**************************************************************************/
static void calcEnerBand(Word16 *real, Word16 *imag, Word32 *ener, Word16 q_en, Word16 *nbSh) {
    Word16 b, normQ, tmp;
    Word16 *ptr_r, *ptr_i, *ptr_sh;
    const Word16 *ptr_b, *ptr_nb, *ptr_nbShQ;
    Word32 *ptrE, L_temp, L_temp2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  2 * SIZE_Word32 + 7 * SIZE_Ptr), "dummy");
#endif
    ptr_r  = real;
    ptr_i  = imag;
    ptrE   = ener;
    ptr_b  = band_region+7;
    ptr_nb = nbCoefBand+7;
    ptr_sh = nbSh;
    ptr_nbShQ = nbShQ;

    FOR(b=START_ILD; b<7; b++)
    {
        L_temp  = L_mult0(*ptr_r, *ptr_r);
        L_temp  = L_mac0(L_temp, *ptr_i, *ptr_i);
        L_temp2 = L_add(1, L_temp);
        *ptrE++ = L_temp2; move32();
        ptr_r++; ptr_i++;
        *ptr_sh++ = q_en; move16();
    }

    FOR(b=7; b<NB_SB; b++)
    {
        normQ = Exp16Array_stereo(*ptr_nb, ptr_r, ptr_i); 
        tmp = sub(maxQ[b],normQ);
        IF(tmp<=0)
        {
            *ptrE++ = calcEnerOneBand0(*ptr_nb, ptr_r, ptr_i); move32();
            *ptr_sh++ = q_en; move16();
        }
        ELSE
        {
            tmp = sub(*ptr_nbShQ, shl(normQ,1));
            *ptrE++ = calcEnerOneBand1(*ptr_nb, ptr_r, ptr_i, tmp); move32();
            *ptr_sh++ = sub(q_en,tmp); move16();
        }
        ptr_r += *ptr_nb;
        ptr_i += *ptr_nb;
        ptr_nb++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

static void smoothEnerWB(Word32 *L_ener, Word16 *q_left_en_band, 
                         Word32 *R_ener, Word16 *q_right_en_band,
                         Word32 *mem_L_ener, Word16 *pre_q_left_en_band, 
                         Word32 *mem_R_ener, Word16 *pre_q_right_en_band);
static Word32 smoothEnerWBOneBand( Word32 enerCur, Word16 *qCur, Word32 memEner, Word16 qPre);
void quantILD(Word16 frame_idx, Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band, Word16 *ILD, 
              Word16 *ILD_q, Word16 *mem_ild_q, Word16 *r1ws_pt);
void quantILD0(Word16 nb4, Word16 nb3, Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band,
              Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, Word16 *r1ws_pt);
static Word16 selectRefineILD(Word16 frame_idx,Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band,
              Word16 *ILD, Word16 *mem_ild_q);
static Word16 quantRefineILD(Word16 Ops, Word16 swb_flag, Word16 ic_flag, Word16 frame_idx, 
              Word16 idx, Word16 *ILD, Word16 *mem_ild_q, Word16 *r1ws_pt);
static Word16 calc_quantILD_abs(Word16 b, Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band,
              Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, Word16 *r1ws_pt);
static Word16 calc_quantILD_diff(Word16 b, Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band,
              Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, Word16 preILDq, Word16 *r1ws_pt);
void quantWholeWB_ITDorIPD(Word16 fb_ITD, Word16 fb_IPD, Word16 *r1ws_pt);

/*************************************************************************
* g722_stereo_encode
*
* G.722 wideband stereo encoder
**************************************************************************/
void g722_stereo_encode(Word16* L_real,
                        Word16* L_imag,
                        Word16* R_real,
                        Word16* R_imag,
                        Word16  q_left,
                        Word16  q_right,
                        Word16* bpt_stereo,
                        void *  ptr, 
                        Word16* frame_idx,
                        Word16  mode,
                        Word16  Ops
                        )
{
    g722_stereo_encode_WORK *w = (g722_stereo_encode_WORK *)ptr;
    Word16 ILD[NB_SB];/* ILD per subband */
    Word16 ILD_q[NB_SB];
    Word16 fac;
    Word16 i, j;
    Word16 flag;
    Word16 idx;
    Word16 *r1ws_pt;
    Word16 fb_ITD;
    Word16 fb_IPD;

    Word16 tmp, tmp2, tmp3;
    Word16 q_left_en,q_right_en;
    Word16 q_left_en_band[20],q_right_en_band[20];

    Word32 L_ener[NB_SB];      
    Word32 R_ener[NB_SB]; 
    Word16 MODE_R2ws_flag, MODE_R5ss_flag, swb_flag;
    Word16 nbBitRefineILD;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((16 + 2 * NB_SB + 2 * 20) * SIZE_Word16 +  (2 * NB_SB) * SIZE_Word32
        + 2 * SIZE_Ptr), "dummy");
#endif
    MODE_R2ws_flag = (sub(mode, MODE_R2ws) == 0);
    MODE_R5ss_flag = (sub(mode, MODE_R5ss) == 0);
    swb_flag = (sub(Ops, 32000)==0);

    /* compute energy in each subband */
    q_left_en  = shl(q_left,1);
    q_right_en = shl(q_right,1);
    calcEnerBand(L_real, L_imag, L_ener, q_left_en,  q_left_en_band); 
    calcEnerBand(R_real, R_imag, R_ener, q_right_en, q_right_en_band);
    fb_ITD = w->fb_ITD; move16();
    fb_IPD = w->fb_IPD; move16();
    r1ws_pt = bpt_stereo;
    /*ILD attack detection in WB*/
    w->num = sub(w->num, 1);
    flag = ild_attack_detect(L_ener, R_ener, w, q_left_en_band, q_right_en_band);
    test();
    tmp = (w->pre_flag && w->num == 0);
    if (tmp)
    {
        flag = 1;move16();
    }
    if(!tmp)
    {
        w->num = 1; move16();
    }

    write_index1(r1ws_pt, flag);
    r1ws_pt += 1;

    IF(flag)/*ILD quantization in 2 frames mode*/
    {
        fac = 2; move16();
        if(sub(flag, w->pre_flag) != 0)
        {
            *frame_idx = 0;move16();
        }
        write_index1(r1ws_pt, *frame_idx);
        r1ws_pt += 1;
        /*calculate and quantize ILD for 10 sub-bands with regular increment (either all even or all odd)*/
        quantILD0(sub(5, swb_flag), add(4,swb_flag), &L_ener[*frame_idx], &R_ener[*frame_idx],
                  &q_left_en_band[*frame_idx], &q_right_en_band[*frame_idx], &ILD[*frame_idx], 
                  &ILD_q[*frame_idx], &w->mem_ild_q[*frame_idx], r1ws_pt);
        r1ws_pt += sub(37, swb_flag);

        /*if it is the first frame of first 2 frames mode,replace the non-transmitted ild by the ild in the adjacent subbands*/
        IF(w->c_flag)
        {
            IF( *frame_idx == 0)
            {
                FOR(i = 0; i < 19 ; i += 2)
                {
                    w->mem_ild_q[i + 1] = w->mem_ild_q[i];move16();
                }
                w->c_flag = 0;move16();
            }
        }
        /*update the memory*/
        IF(sub(*frame_idx, 1) == 0) 
        {
            FOR(i = 0; i < 20; i++)
            {
                tmp2 = norm_l(L_ener[i]);
                tmp3 = norm_l(R_ener[i]);
                w->mem_L_ener[i] = L_shl(L_ener[i],tmp2);move32();
                w->mem_R_ener[i] = L_shl(R_ener[i],tmp3);move32();
                w->pre_q_left_en_band[i]  = add(q_left_en_band[i],tmp2); move16();
                w->pre_q_right_en_band[i] = add(q_right_en_band[i],tmp3); move16();
            }
        }
    }
    ELSE /*ILD quantization in 4 frames mode*/
    {
        fac = 4;       move16();
        w->c_flag = 1; move16();
        if(sub(flag, w->pre_flag) != 0)
        {
            *frame_idx = 0; move16();
        }
        write_index2(r1ws_pt, *frame_idx);
        r1ws_pt += 2;

        IF(w->frame_flag_wb == 0)
        {
            FOR(i = 0; i < 20; i++)
            {
                w->pre_q_left_en_band[i]  = add(q_left_en_band[i],norm_l(L_ener[i])); move16();
                w->pre_q_right_en_band[i] = add(q_right_en_band[i],norm_l(R_ener[i])); move16();
            }
            w->frame_flag_wb = 1;move16();
        }
        smoothEnerWB(L_ener, q_left_en_band, R_ener, q_right_en_band, w->mem_L_ener, 
                     w->pre_q_left_en_band, w->mem_R_ener, w->pre_q_right_en_band);
        /*calculate and quantize ILD for each sub band*/
        quantILD(*frame_idx, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                 ILD, ILD_q, w->mem_ild_q, r1ws_pt);
        r1ws_pt += 22;
        /*inter-channel diference selection*/
        IF(w->ic_flag) /*whole wideband IC is selected*/
        {
            write_index1(r1ws_pt,1);  
            r1ws_pt += 1;
            idx = 15; move16();
            write_index4(r1ws_pt,idx);
            r1ws_pt += 4;
            write_index2(r1ws_pt,w->ic_idx);
            r1ws_pt += 2;
        }
        ELSE
        {
            quantWholeWB_ITDorIPD(fb_ITD, fb_IPD, r1ws_pt);
            r1ws_pt += 5;
        }

        /*ILD refinement */
        idx = selectRefineILD(*frame_idx, L_ener, R_ener, q_left_en_band, 
                              q_right_en_band, ILD, w->mem_ild_q);
        write_index2(r1ws_pt, idx);
        r1ws_pt += 2;

        /*The number of bits used for ILD refinement is determined by IC flag*/
        nbBitRefineILD = quantRefineILD(Ops, swb_flag, w->ic_flag, *frame_idx, 
                                        idx, ILD, w->mem_ild_q, r1ws_pt);
        r1ws_pt  += nbBitRefineILD ;
    }

    w->pre_flag = flag; move16();
    *frame_idx = add(*frame_idx, 1) % fac; logic16();
    IF(swb_flag)
    {
        write_index1(r1ws_pt, w->SWB_ILD_mode); 
        r1ws_pt += 1;
    }

    bpt_stereo += 39;

    /*write the IPD information into the bitstream*/ 
    IF (MODE_R2ws_flag)
    {
        FOR(j=IPD_SYN_START; j<=IPD_SYN_WB_END_WB; j++)
        {
            write_index5( bpt_stereo, w->idx[j]);
            bpt_stereo += 5;
        }
    }
    IF(swb_flag)
    {
        FOR(j=IPD_SYN_START; j<IPD_SYN_WB_END_SWB; j++)
        {
            write_index5( bpt_stereo, w->idx[j]);
            bpt_stereo += 5;
        }
        IF(w->SWB_ILD_mode == 0)
        {
            write_index4( bpt_stereo, w->idx[IPD_SYN_WB_END_SWB]);
            bpt_stereo += 4;
        }
        ELSE
        {
            write_index5( bpt_stereo, w->idx[IPD_SYN_WB_END_SWB]);
            bpt_stereo += 5;
        }
    }
    /*write the IPD information into the bitstream for R5ss*/
    IF(MODE_R5ss_flag)
    {  
        bpt_stereo += 5;
        if (w->SWB_ILD_mode == 0)
        {
            bpt_stereo += 1;
        }

        FOR(j = IPD_SYN_WB_END_SWB + 1; j <= IPD_SYN_SWB_END_SWB; j++)
        {
            write_index5( bpt_stereo, w->idx[j]);
            bpt_stereo += 5;
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* quantRefineILD
*
* refinement of ILD quantization
**************************************************************************/
static Word16 quantRefineILD(Word16 Ops, Word16 swb_flag, Word16 ic_flag, Word16 frame_idx, 
                             Word16 idx, Word16 *ILD, Word16 *mem_ild_q, Word16 *r1ws_pt)
{
    Word16 i, nbBitRefineILD;
    Word16 parityFrame_idx, parityIdx, flagNbBand, incBand, b0;
    Word16 diff, idx1;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((7) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
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
    /* The number of bits used for ILD refinement is determined by IC flag */
    IF(ic_flag)
    { /* case ic transmitted */
        nbBitRefineILD = 4; move16();
        IF(sub(Ops, 16000) == 0)
        {
            /* WB and case ic transmitted */
            nbBitRefineILD = add(nbBitRefineILD, 1);
            /* only 2 subbands to quantize on 3 and 2 bits */ 
            diff = sub(ILD[b0], mem_ild_q[b0]);
            idx1 = searchIdxPWQU_3seg_3bits(diff);
            mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q3[idx1]); move16();
            write_index3(r1ws_pt, idx1);
            r1ws_pt += 3;

            b0 = add(b0, incBand);
            diff = sub(ILD[b0], mem_ild_q[b0]);
            idx1 = searchSegQ_2bits (diff);
            mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q2[idx1]);
            write_index2(r1ws_pt, idx1);
        } /* end WB and case ic transmitted */
        ELSE
        { /* SWB and case ic transmitted */
            /* only 2 subbands to quantize both on 2 bits */ 
            diff = sub(ILD[b0], mem_ild_q[b0]);
            idx1 = searchSegQ_2bits (diff);
            mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q2[idx1]);
            write_index2(r1ws_pt, idx1);
            r1ws_pt += 2;

            b0 = add(b0, incBand);
            diff = sub(ILD[b0], mem_ild_q[b0]);
            idx1 = searchSegQ_2bits (diff);
            mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q2[idx1]);
            write_index2(r1ws_pt, idx1);
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
                diff = sub(ILD[b0], mem_ild_q[b0]);
                idx1 = searchIdxPWQU_3seg_3bits(diff);
                mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q3[idx1]); move16();
                write_index3(r1ws_pt, idx1);
                r1ws_pt += 3;
                b0 = add(b0, incBand);
                FOR(i = 0; i < 2; i++)
                {
                    diff = sub(ILD[b0], mem_ild_q[b0]);
                    idx1 = searchSegQ_2bits (diff);
                    mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q2[idx1]); move16();
                    write_index2(r1ws_pt, idx1);
                    r1ws_pt += 2;
                    b0 = add(b0, incBand);
                }
            } /* end case 3 subbands quantized first on 3 bits , last two subbands on 2 bits */
            ELSE
            { /* case 2 subbands quantized first on 4 bits, last on 3 bits */
                diff = sub(ILD[b0],mem_ild_q[b0]);
                idx1 = searchIdxPWQU_3seg_4bits( diff); move16();
                mem_ild_q[b0] =add(mem_ild_q[b0], tab_ild_q4[idx1]); move16();
                write_index4(r1ws_pt, idx1);
                r1ws_pt += 4;
                b0 = add(b0, incBand);
        
                diff = sub(ILD[b0], mem_ild_q[b0]);
                idx1 = searchIdxPWQU_3seg_3bits(diff);
                mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q3[idx1]); move16();
                write_index3(r1ws_pt, idx1);
            } /* end case 2 subbands quantized first on 4 bits, last on 3 bits */
        } /* end  WB and case ic non transmitted */
        ELSE
        { /* SWB and case ic non transmitted */
            IF( flagNbBand == 0)
            {  /* case 3 subband quantized all on 2 bits */
                FOR(i = 0; i < 3; i++)
                {
                    diff = sub(ILD[b0], mem_ild_q[b0]);
                    idx1 = searchSegQ_2bits (diff);
                    mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q2[idx1]); move16();
                    write_index2(r1ws_pt, idx1);
                    r1ws_pt += 2;
                    b0 = add(b0, incBand);
                }
            } /* end case 3 subband quantized all on 2 bits */
            ELSE
            { /* case 2 subband quantized all on 3 bits */
                FOR(i = 0; i < 2; i++)
                {
                    diff = sub(ILD[b0], mem_ild_q[b0]);
                    idx1 = searchIdxPWQU_3seg_3bits(diff);
                    mem_ild_q[b0] = add(mem_ild_q[b0], tab_ild_q3[idx1]); move16();
                    write_index3(r1ws_pt, idx1);
                    r1ws_pt += 3;
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
* selectRefineILD
*
* selection of ILD group to be refined
**************************************************************************/
static Word16 selectRefineILD(Word16 frame_idx, Word32 *L_ener, Word32 *R_ener, 
                              Word16 *q_left_en_band, Word16 *q_right_en_band,
                              Word16 *ILD, Word16 *mem_ild_q)
{
    Word16 ild_diff_sum[4], *ptr1;
    Word16 max_diff_sum;
    Word16 idx, i, k, j, tmp;
    const Word16 *ptr0, *ptr2;
  
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((6+4) * SIZE_Word16 +  (0) * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif

    ptr0 = (Word16 *)(band_stat2 + shl(frame_idx, 4));
    ptr1 = ild_diff_sum; 
    ptr2 = (Word16 *)(nbBand_stat + shl(frame_idx, 2)); 
    FOR(k = 0; k < 4; k++)
    {
        tmp = 0; move16();
        FOR(j = 0; j < *ptr2; j++)
        {
            i= *ptr0++; move16();
            ILD[i] = ild_calculation(L_ener[i],R_ener[i],q_left_en_band[i], q_right_en_band[i]); move16();
            tmp = add(tmp, mult(abs_s(sub(ILD[i],mem_ild_q[i])), band_region_ref[i]));
        }
        *ptr1++ = mult(tmp, band_num[frame_idx][k]); move16();
        ptr2++;
    }

    max_diff_sum = ild_diff_sum[0];move16();
    idx = 0;
    FOR (i = 1; i< 4; i++)
    {
        if(sub(max_diff_sum, ild_diff_sum[i]) < 0)
        {
            idx = i;move16();
        }
        max_diff_sum = s_max(max_diff_sum, ild_diff_sum[i]);
    }

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (idx);
}

/*************************************************************************
* quantWholeWB_ITDorIPD
*
* quantize whole WB parameter ITD or IPD
**************************************************************************/
void quantWholeWB_ITDorIPD(Word16 fb_ITD, Word16 fb_IPD, Word16 *r1ws_pt)
{
    Word16 idx;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    idx = add(fb_ITD, 16+7); /* case: whole wideband ITD is selected: index=  MSB "1" + 4 LSB "fb_ITD+7" */
    IF(!fb_ITD) 
    { /* case whole wideband IPD is selected index=  MSB "0" + 4 LSB "idx_quand_fb_IPD" */
        idx = searchIdxQU(fb_IPD,paramQuantPhase+5, tab_phase_q4); move16();
    }
    /* write 5 bits index */
    write_index5( r1ws_pt, idx);

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif

    return;
}

/*************************************************************************
* quantILD0
*
* calculate and quantize sub band ILD with regular subband increment (+2)
**************************************************************************/
void quantILD0(Word16 nb4, Word16 nb3, Word32 *L_ener, Word32 *R_ener, 
               Word16 *q_left_en_band, Word16 *q_right_en_band,
               Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, Word16 *r1ws_pt)
{
    Word16 idx, b, diff, preILDq;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((4) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    /*calculate and quantize ILD on first sub-band with 5 bits */
    *ILD = ild_calculation(*L_ener,*R_ener,*q_left_en_band,*q_right_en_band); move16(); 
    idx = searchIdxPWQU_5seg_5bits( *ILD); move16(); 
    preILDq = tab_ild_q5[idx]; move16();

    *ILD_q = preILDq; move16();
    *mem_ild_q = preILDq;move16();
    write_index5(r1ws_pt, idx);
    r1ws_pt += 5;
    FOR(b = 0; b < nb4; b++)
    {
        ILD             += 2;
        mem_ild_q       += 2;
        L_ener          += 2;
        R_ener          += 2;
        q_left_en_band  += 2;
        q_right_en_band += 2;

        /*calculate and quantize ILD on following  sub-bands with 4 bits */
        *ILD = ild_calculation(*L_ener,*R_ener,*q_left_en_band,*q_right_en_band); move16(); 
        diff = sub(*ILD, preILDq );
        idx = searchIdxPWQU_3seg_4bits( diff);
        write_index4(r1ws_pt, idx);
        r1ws_pt += 4;
        preILDq = add(preILDq, tab_ild_q4[idx]); move16();
        *ILD_q = preILDq; move16();
        *mem_ild_q= preILDq;move16();
    }
    FOR(b=0; b < nb3; b ++)
    {
        ILD             += 2;
        mem_ild_q       += 2;
        L_ener          += 2;
        R_ener          += 2;
        q_left_en_band  += 2;
        q_right_en_band += 2;

        *ILD = ild_calculation(*L_ener,*R_ener,*q_left_en_band,*q_right_en_band); move16(); 
        diff = sub(*ILD, preILDq );
        idx = searchIdxPWQU_3seg_3bits(diff);
        write_index3(r1ws_pt, idx);
        r1ws_pt += 3;
        preILDq = add(preILDq, tab_ild_q3[idx]); move16();
        *ILD_q = preILDq; move16();
        *mem_ild_q= preILDq;move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* quantILD
*
* calculate and quantize sub band ILD with irregular  subband increment 
* (subband index read in array band_index)
**************************************************************************/
void quantILD(Word16 frame_idx, Word32 *L_ener, Word32 *R_ener, 
              Word16 *q_left_en_band, Word16 *q_right_en_band,
              Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, Word16 *r1ws_pt)
{
    Word16 b, preILDq;
    const Word16 *ptrBand;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif

    ptrBand = &band_index[frame_idx][0];
    /* 1st band in category absolute quantization */
    b = *ptrBand++; move16();
    preILDq = calc_quantILD_abs(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                ILD, ILD_q, mem_ild_q, r1ws_pt);
    r1ws_pt += 5;

    /* 2nd band in category  differential quantization  */
    b = *ptrBand++; move16();
    preILDq = calc_quantILD_diff(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                 ILD, ILD_q, mem_ild_q, preILDq, r1ws_pt);
    r1ws_pt += 4;

    b = *ptrBand++; move16();
    IF(sub(frame_idx,2) < 0) 
    {        /* frame_idx= 0 or 1 */
        /* 3rd band in category  differential quantization  */
        preILDq = calc_quantILD_diff(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                     ILD, ILD_q, mem_ild_q, preILDq, r1ws_pt);
        r1ws_pt += 4;

        /*4th band in category absolute quantization */
        b = *ptrBand++; move16();
        preILDq = calc_quantILD_abs(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                    ILD, ILD_q, mem_ild_q, r1ws_pt);
        r1ws_pt += 5;
    }
    ELSE
    {        /* frame_idx= 2 or 3 */
        /* 3rd band in category  absolutequantization  */
        preILDq = calc_quantILD_abs(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                    ILD, ILD_q, mem_ild_q, r1ws_pt);
        r1ws_pt += 5;

        /*4th band in category differential  quantization */
        b = *ptrBand++; move16();
        preILDq = calc_quantILD_diff(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                     ILD, ILD_q, mem_ild_q, preILDq, r1ws_pt);
        r1ws_pt += 4;
    }

    b = *ptrBand++; move16();
    /* last band in category differential quantization  */
    preILDq = calc_quantILD_diff(b, L_ener, R_ener, q_left_en_band, q_right_en_band, 
                                 ILD, ILD_q, mem_ild_q, preILDq, r1ws_pt);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_quantILD_abs
*
* ILD computation of subband b by absolute quantization
**************************************************************************/
static Word16 calc_quantILD_abs(Word16 b, Word32 *L_ener, Word32 *R_ener, 
                                Word16 *q_left_en_band, Word16 *q_right_en_band,
                                Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, 
                                Word16 *r1ws_pt)
{
    Word16 idx, preILDq;
    Word16 *ptrILD, *ptrILDq, *ptrMemILDq;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    ptrILD     = ILD+b; add(0,0);
    ptrILDq    = ILD_q+b; add(0,0);
    ptrMemILDq = mem_ild_q+b; add(0,0);
    *ptrILD = ild_calculation(L_ener[b],R_ener[b],q_left_en_band[b], q_right_en_band[b]); move16();
    idx = searchIdxPWQU_5seg_5bits( *ptrILD); move16();
    write_index5(r1ws_pt, idx);
    preILDq = tab_ild_q5[idx]; move16();
    *ptrILDq    = preILDq; move16();
    *ptrMemILDq = preILDq; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(preILDq);
}

/*************************************************************************
* calc_quantILD_diff
*
* ILD computation of subband b by differential quantization
**************************************************************************/
static Word16 calc_quantILD_diff(Word16 b, Word32 *L_ener, Word32 *R_ener, 
                                 Word16 *q_left_en_band, Word16 *q_right_en_band,
                                 Word16 *ILD, Word16 *ILD_q, Word16 *mem_ild_q, 
                                 Word16 preILDq, Word16 *r1ws_pt)
{
    Word16 idx, diff;
    Word16 *ptrILD, *ptrILDq, *ptrMemILDq;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    ptrILD     = ILD+b; add(0,0);
    ptrILDq    = ILD_q+b; add(0,0);
    ptrMemILDq = mem_ild_q+b; add(0,0);
    *ptrILD = ild_calculation(L_ener[b],R_ener[b],q_left_en_band[b], q_right_en_band[b]); move16();
    diff = sub(*ptrILD, preILDq);
    idx = searchIdxPWQU_3seg_4bits( diff); move16();
    preILDq = add(preILDq, tab_ild_q4[idx]);
    write_index4(r1ws_pt, idx);
    *ptrILDq    = preILDq; move16();
    *ptrMemILDq = preILDq; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(preILDq);
}

/*************************************************************************
* smoothEnerWB
*
* smoothing of Left and Right channels enery for all WB subbands
**************************************************************************/
static void smoothEnerWB(Word32 *L_ener, Word16 *q_left_en_band, 
                         Word32 *R_ener, Word16 *q_right_en_band,
                         Word32 *mem_L_ener, Word16 *pre_q_left_en_band, 
                         Word32 *mem_R_ener, Word16 *pre_q_right_en_band)
{
    Word16 b;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    FOR(b = 0; b < NB_SB; b++)
    {
        L_ener[b] = smoothEnerWBOneBand(L_ener[b] , &q_left_en_band[b], 
                                        mem_L_ener[b], pre_q_left_en_band[b]); move32();
        R_ener[b] = smoothEnerWBOneBand(R_ener[b] , &q_right_en_band[b], 
                                        mem_R_ener[b], pre_q_right_en_band[b]); move32();
    }
    /* update memory */
    FOR(b = 0; b < NB_SB; b++)
    {
        mem_L_ener[b]          = L_ener[b]; move32();
        mem_R_ener[b]          = R_ener[b]; move32();
        pre_q_left_en_band[b]  = q_left_en_band[b];move16();
        pre_q_right_en_band[b] = q_right_en_band[b];move16();
    }    
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* smoothEnerWB
*
* smoothing of Left and Right channels enery for one WB subband
**************************************************************************/
static Word32 smoothEnerWBOneBand( Word32 enerCur, Word16 *qCur, Word32 memEner, Word16 qPre)
{
    Word16 nbSh, diffQ;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((2) * SIZE_Word16 +  (0) * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif

    nbSh  = norm_l(enerCur);
    enerCur = L_shl(enerCur,nbSh );
    nbSh  = add(*qCur, nbSh); 
    diffQ = sub(qPre, nbSh);
    /*smooth the energy between two consecutive frames*/
    enerCur = L_add(L_shr(memEner,s_max(1,add(diffQ ,1))),L_shr( enerCur, s_max(sub(1, diffQ),1)) ); 
    *qCur = s_min(nbSh, qPre); move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(enerCur);
}
#endif /* LAYER_STEREO */
