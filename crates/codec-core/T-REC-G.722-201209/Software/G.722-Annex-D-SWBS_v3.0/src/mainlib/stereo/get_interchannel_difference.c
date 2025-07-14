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

static void get_ic(Word16 Ipd, Word16 Itd, Word16 flagHighEner, 
                   Word16 phase_mean_std_sm_ct, Word16 *ic_idx, Word16 *ic_flag);
static void calcPhaseStd(Word16 *temp_phase_ipd, Word16 *phase_mean_buf1, 
                         Word16 *phase_num, Word16 *phase_mean_std_sm_ct, Word16 *pos1);
static Word16 calcPhaseMean(Word16 *phase_mean_buf, Word16 mean_ipd, 
                            Word16 *phase_mean_std_sm, Word16 *f_num, 
                            Word16 *ipd_mean_sm, Word32 ipd_reg_num_sm,
                            Word16 en_ratio_sm);
static void updateIpd(Word32 *mean_ipd, Word16 *pre_ipd_mean, Word16 *ipd_num, 
                      Word16 *ipd_reg_num, Word16 *ipd_reg_num_sm, Word16 *phase_mean_buf);
static Word32 excludeOutlier(Word32 mean_ipd, Word16 *temp_phase_ipd, Word16 pre_Ipd);
static Word16 getGlobalPhaseStd(Word32 mean_ipd, Word16 *temp_phase_ipd, Word32 *energy_bin_sm);
static Word32 getWeightedMeanGlobalPhase (Word32 *energy_bin_sm, Word16 *temp_phase_ipd);
static Word16 selectITDmain(Word16 nb_pos, Word16 nb_neg, Word32 std_itd_pos, Word32 std_itd_neg, 
                            Word16 mean_itd_pos, Word16 mean_itd_neg, Word16 pre_Itd);
static Word16 checkMeaningfulITD(Word32 std_itd_sm, Word32 nb_idx_sm, Word16 pre_Itd, Word16 flagITDstd);
static Word16 smoothMeanITD(Word16 mean_itd_pos, Word16 *pre_itd);
static void smoothITD(Word16 nb_inst, Word16 std_itd_inst,  Word16 *std_itd, 
                      Word16 flagHighEner,Word16 *nb, Word16 *mean_itd, Word16 mean_itd_inst, 
                      Word32 *Crxyt, Word32 *Crxyt2, Word32 *Cixyt, Word32 *Cixyt2,
                      Word16 *std_itd_sm, Word16 *nb_idx_sm);
static void calcStandITD(Word16 *temp_phase, Word16 *itd, 
                         Word16 nb_pos, Word16 mean_itd_pos, Word16 *std_itd_pos2, 
                         Word16 nb_neg, Word16 mean_itd_neg, Word16 *std_itd_neg2);
static Word16 getITDstd(Word32 std_itd, Word16 n, Word16 nb);
static Word32 updateMemEner(Word32 mem_energy,  Word32 energy);
static void udpdateEnerBin(Word16 f_num , Word16 n, Word32 *energy_bin, Word32 *energy_bin_sm);
static void udpdateEner_ratio_sm(Word32 en_l, Word32 en_r, Word16 *en_ratio_sm);
static void normalizeEnerBin(Word16 n, Word32 en_l, Word32 en_r, Word32 *energy_bin);
static void calcEnerITD(Word16 n, Word32 *energyL, Word32 *energyR, 
                        Word32 *energy_bin, Word16 q_left, Word16 q_right, 
                        Word32 *Crxyt, Word32 *Cixyt, Word32 *Crxyt2, Word32 *Cixyt2,
                        Word16 *temp_phase, Word16 *temp_phase_ipd, 
                        Word16 *L_real, Word16 *L_imag, Word16 *R_real, Word16 *R_imag);
static void calcMeanITD(Word16 *temp_phase, Word16 *itd, Word16 *mean_itd_pos, 
                        Word16 *mean_itd_neg, Word16 *nb_pos2, Word16 *nb_neg2);
static Word16 getMeanITD(Word32 sum_itd, Word16 nb);

/*************************************************************************
* get_interchannel_difference
*
* calculate whole wideband ITD IPD and IC
**************************************************************************/
void get_interchannel_difference(g722_stereo_encode_WORK *w,
                                 Word16 L_real[],
                                 Word16 L_imag[],
                                 Word16 q_left,
                                 Word16 R_real[],
                                 Word16 R_imag[],
                                 Word16 q_right,
                                 Word16 *ic_idx,
                                 Word16 *ic_flag
                                 )
{
    Word16 i;
    Word16 flagITDstd, flagHighEner;
    Word16 flagPos, flagNeg;
    Word32 *Crxyt, *Cixyt, *Crxyt2, *Cixyt2;

    Word16 nb_pos, nb_neg, nb_pos_inst, nb_neg_inst;
    Word16 Itd, Ipd;
    Word16 mean_itd_pos, mean_itd_neg, mean_itd_neg_inst, mean_itd_pos_inst;
    Word16 std_itd_pos_inst, std_itd_neg;
    Word16 std_itd_pos, std_itd_neg_inst;
    Word32 mean_ipd;
    Word16 std_ipd;

    /* input */
    Word32 energyL;
    Word32 energyR;
    Word16 temp_phase[STARTBANDITD+BANDITD];
    Word16 temp_phase_inst[STARTBANDITD+BANDITD];
    Word16 temp_phase_ipd[STARTBANDITD+BANDITD];
    Word16 itd[STARTBANDITD+BANDITD];
    Word16 itd_inst[STARTBANDITD+BANDITD];
    Word32 phase_mean_std_mean,phase_mean_std_std;
    Word32 energy_bin[STARTBANDITD + BANDITD], en_l, en_r;

#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((22 + 5 * (STARTBANDITD+BANDITD)) * SIZE_Word16 + 
        (11 + (STARTBANDITD + BANDITD)) * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    phase_mean_std_mean = 0; move32();
    phase_mean_std_std = 0;  move32();
    en_l = 0; move32();
    en_r = 0; move32();

    q_left  = sub(q_left, 11);
    q_right = sub(q_right, 11);

    /* computation and (weak and strong) smoothing of cross spectrum */
    energyL = 0; move32();
    energyR = 0; move32();
    Crxyt  = w->Crxyt + STARTBANDPHA;
    Cixyt  = w->Cixyt + STARTBANDPHA;
    Crxyt2 = w->Crxyt2 + STARTBANDPHA;
    Cixyt2 = w->Cixyt2 + STARTBANDPHA;

    calcEnerITD(STARTBANDPHA+BANDPHA-1, &energyL, &energyR, &energy_bin[1], 
                q_left, q_right, Crxyt, Cixyt, Crxyt2, Cixyt2, &temp_phase[1], 
                &temp_phase_ipd[1],&L_real[1], &L_imag[1], &R_real[1], &R_imag[1]);

    en_l = energyL; move32();
    en_r = energyR; move32();
    Crxyt  += BANDPHA;
    Cixyt  += BANDPHA;
    Crxyt2 += BANDPHA;
    Cixyt2 += BANDPHA;

    calcEnerITD(STARTBANDITD+BANDITD-(STARTBANDPHA+BANDPHA), &energyL, &energyR, &energy_bin[STARTBANDPHA+BANDPHA], 
                q_left, q_right, Crxyt, Cixyt, Crxyt2, Cixyt2, &temp_phase[STARTBANDPHA+BANDPHA], 
                &temp_phase_ipd[STARTBANDPHA+BANDPHA], &L_real[STARTBANDPHA+BANDPHA], &L_imag[STARTBANDPHA+BANDPHA],
                &R_real[STARTBANDPHA+BANDPHA], &R_imag[STARTBANDPHA+BANDPHA]);

    FOR(i=STARTBANDPHA; i<STARTBANDITD+BANDITD; i++) 
    {
        temp_phase_inst[i] =negate(temp_phase_ipd[i]); move16();
    }

    /*get weighting factor for full band IPD estimation*/
    normalizeEnerBin(STARTBANDPHA+BANDPHA-1, en_l, en_r, &energy_bin[1]);

    udpdateEner_ratio_sm(en_l, en_r, &w->en_ratio_sm); 

    udpdateEnerBin(w->f_num ,STARTBANDPHA+BANDPHA-1, &energy_bin[1], &w->energy_bin_sm[1]);

    /* get positive and negative mean ITD with weak and strong smoothing */
    calcMeanITD(temp_phase, itd, &mean_itd_pos, &mean_itd_neg, &nb_pos, &nb_neg);
    calcMeanITD(temp_phase_inst, itd_inst, &mean_itd_pos_inst, &mean_itd_neg_inst, &nb_pos_inst, &nb_neg_inst);

    /* update channel energies */
    w->mem_energyL = updateMemEner(w->mem_energyL, energyL); move32();
    w->mem_energyR = updateMemEner(w->mem_energyR, energyR); move32();

    /* Non meaningfull ITD for low energy stereo signal /*/
    /* flagITDstd = 0 if no low energy channel; #0 otherwise */
    flagITDstd = 0; move16();
    if(L_sub(w->mem_energyL, 35) < 0) 
    {
        flagITDstd = add(flagITDstd ,1);
    }
    if(L_sub(w->mem_energyR, 35) < 0)
    {
        flagITDstd = add(flagITDstd ,1);
    }

    /*compute if high energy >100 in both channels = 1;  otherwise = 0  */
    flagHighEner = 0; move16();
    if(L_sub(w->mem_energyL, 354) > 0 )
    {
        flagHighEner = add(flagHighEner, 1);
    }
    if(L_sub(w->mem_energyR, 354) > 0 )
    {
        flagHighEner = add(flagHighEner, 1);
    }
    flagHighEner = shr(flagHighEner, 1);

    /* ITD standard deviation for positive and negative mean ITD with weak and strong smoothing */
    IF(flagITDstd == 0) 
    {
        calcStandITD(&temp_phase[STARTBANDITD], &itd[STARTBANDITD], nb_pos, mean_itd_pos, 
                     &std_itd_pos, nb_neg, mean_itd_neg, &std_itd_neg );
    }
    ELSE
    {
        nb_pos = 0; move16();
        nb_neg = 0; move16();
        std_itd_pos = 1792; move16();
        std_itd_neg = 1792; move16();
    }
    calcStandITD(&temp_phase_inst[STARTBANDITD], &itd_inst[STARTBANDITD], 
                 nb_pos_inst, mean_itd_pos_inst, &std_itd_pos_inst, 
                 nb_neg_inst, mean_itd_neg_inst, &std_itd_neg_inst);

    /* If weak smoothing positive ITD is stronger than strong smoothing positive ITD */
    /* replace strong smoothing positive ITD by weak smoothing positive ITD */
    Crxyt  = w->Crxyt;
    Cixyt  = w->Cixyt;
    Crxyt2 = w->Crxyt2;
    Cixyt2 = w->Cixyt2;
    smoothITD(nb_pos_inst, std_itd_pos_inst,  &std_itd_pos, flagHighEner, 
              &nb_pos, &mean_itd_pos, mean_itd_pos_inst, Crxyt, Crxyt2, 
              Cixyt, Cixyt2, &w->std_itd_pos_sm, &w->nb_idx_pos_sm );

    /* If weak smoothing negative ITD is stronger than strong smoothing negative ITD */
    /* replace strong smoothing negative ITD by weak smoothing negative ITD */
    smoothITD(nb_neg_inst, std_itd_neg_inst, &std_itd_neg, flagHighEner,
              &nb_neg, &mean_itd_neg, mean_itd_neg_inst, Crxyt, Crxyt2, 
              Cixyt, Cixyt2, &w->std_itd_neg_sm,  &w->nb_idx_neg_sm );

    /* smoothing of positive and negative ITD */
    mean_itd_pos = smoothMeanITD(mean_itd_pos, &w->pre_itd_pos); move16();
    mean_itd_neg = smoothMeanITD(mean_itd_neg, &w->pre_itd_neg); move16();

    /* non meaningful ITD set to zero */
    flagPos = checkMeaningfulITD(w->std_itd_pos_sm, w->nb_idx_pos_sm, w->pre_Itd, flagITDstd);
    if(flagPos !=0)
    {
        mean_itd_pos = 0; move16();
    }
    flagNeg = checkMeaningfulITD(w->std_itd_neg_sm, w->nb_idx_neg_sm, w->pre_Itd, flagITDstd);
    if(flagNeg !=0 ) 
    {
        mean_itd_neg = 0; move16();
    }

    /* selection of main ITD among the positive and negative */
    Itd = selectITDmain(nb_pos, nb_neg, std_itd_pos, std_itd_neg, mean_itd_pos, mean_itd_neg, w->pre_Itd);

    /* get weighted mean whole wideband IPD */
    mean_ipd = getWeightedMeanGlobalPhase(w->energy_bin_sm, temp_phase_ipd); 

    /* get standard deviation of whole wideband IPD */
    std_ipd = getGlobalPhaseStd(mean_ipd, temp_phase_ipd, w->energy_bin_sm);

    /* exclude outliers from mean whole wideband IPD and standard deviation */
    IF (sub(std_ipd, PI_D8_4096) > 0)
    {
        mean_ipd = excludeOutlier(mean_ipd, temp_phase_ipd, w->pre_Ipd);
    }

    if (Itd != 0)
    {
        mean_ipd = 0; move32();
    }

    /* avoid instability of whole wideband IPD */
    updateIpd(&mean_ipd, &w->pre_ipd_mean, &w->ipd_num, &w->ipd_reg_num,
              &w->ipd_reg_num_sm, &w->phase_mean_buf[w->pos1]);

    /* get final whole wideband IPD */
    Ipd = calcPhaseMean(w->phase_mean_buf, extract_l(mean_ipd), &w->phase_mean_std_sm,
                        &w->f_num, &w->ipd_mean_sm, w->ipd_reg_num_sm, w->en_ratio_sm);
    w->pre_Itd = Itd; move16();
    w->pre_Ipd = Ipd; move16();

    /* compute non-weighted mean IPD and standard deviation for IC computation */
    calcPhaseStd(temp_phase_ipd, w->phase_mean_buf1, &w->phase_num, 
                 &w->phase_mean_std_sm_ct, &w->pos1);

    /* get whole wideband IC parameter */
    get_ic(Ipd, Itd, flagHighEner, w->phase_mean_std_sm_ct, ic_idx,ic_flag); 
    w->fb_ITD = Itd;
    w->fb_IPD = Ipd;
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* get_ic
*
* compute the IC parameter
**************************************************************************/
static void get_ic(Word16 Ipd, Word16 Itd, Word16 flagHighEner, 
                   Word16 phase_mean_std_sm_ct, Word16 *ic_idx, Word16 *ic_flag) 
{ 
    Word16 ic_flag2, ic_idx2; 
    Word16 i;  
    const Word16 *ptr; 
    Word16 flag; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 + 1 * SIZE_Ptr), "dummy");
#endif
    ic_flag2 = 0; move16(); 
    IF(flagHighEner !=0) 
    { 
        flag = add(abs_s(Ipd), abs_s(Itd)); 
        IF(flag == 0) 
        { 
            IF(sub(phase_mean_std_sm_ct, 819) >=0) /* 0.2 */ 
            { 
                ic_flag2 = 1; move16(); 
                ptr = threshPhaseMeanStdSMct; 
                ic_idx2 = 3; move16(); 
                FOR(i=0; i<3; i++) 
                { 
                    if(sub(phase_mean_std_sm_ct,*ptr++) >= 0) 
                    { 
                        ic_idx2 = sub(ic_idx2,1); 
                    } 
                } 
                *ic_idx = ic_idx2; move16(); 
            } 
        } 
    } 
    *ic_flag = ic_flag2; move16(); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return; 
} 

/*************************************************************************
* calcPhaseStd
*
* Calculate the standard deviation of phase
**************************************************************************/
static void calcPhaseStd(Word16 *temp_phase_ipd, Word16 *phase_mean_buf1, Word16 *phase_num, 
                         Word16 *phase_mean_std_sm_ct, Word16 *pos1) 
{ 
    Word32 sum_ipd; 
    Word16 phase_mean_mean, phase_mean_std, mean_ipd; 
    Word16 i; 
    Word32 Acc; 
    sum_ipd = L_mult(3277, temp_phase_ipd[STARTBANDPHA]); 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 + 2 * SIZE_Word32), "dummy");
#endif
    FOR(i = STARTBANDPHA + 1; i< STARTBANDPHA + PHA; i++) 
    { 
        sum_ipd = L_mac(sum_ipd, 3277, temp_phase_ipd[i]); 
    } 
    mean_ipd = round_fx(sum_ipd); 
    phase_mean_buf1[*pos1] = mean_ipd; move16(); 
    *pos1 = add(*pos1, 1); 
    if(sub(*pos1, IPD_FNUM) >= 0) 
    { 
        *pos1 = 0; move16(); 
    } 
    Acc = L_mult((Word16)phase_mean_buf1[0], 3277); 
    FOR(i = 1; i < IPD_FNUM; i ++) 
    { 
        Acc = L_mac(Acc, 3277, (Word16)phase_mean_buf1[i]); 
    } 
    phase_mean_mean = round_fx(Acc); 
    phase_mean_std = round_fx(L_abs(L_sub(sum_ipd, Acc))); 
    i = *phase_num; move16(); 
    IF(sub(phase_mean_std, 12) <= 0)/* 0.003 Q12 */ 
    { 
        i = add(i, 1); 
    } 
    ELSE 
    { 
        i=0; move16(); 
    } 
    IF (sub(i, 5) > 0) 
    { 
        *phase_mean_std_sm_ct = phase_mean_std; move16(); 
        i=5; move16(); 
    } 
    ELSE 
    { 
        Acc = L_mult(*phase_mean_std_sm_ct, 32512); 
        Acc = L_mac(Acc, phase_mean_std, 256); 
        *phase_mean_std_sm_ct = round_fx(Acc); 
    } 
    *phase_num = i; move16(); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return; 
} 

/*************************************************************************
* calcPhaseMean
*
* Calculate the mean of the phase
**************************************************************************/
static Word16 calcPhaseMean(Word16 *phase_mean_buf, Word16 mean_ipd, 
                            Word16 *phase_mean_std_sm, Word16 *f_num, 
                            Word16 *ipd_mean_sm, Word32 ipd_reg_num_sm, 
                            Word16 en_ratio_sm)
{ 
    Word16 Ipd, i; 
    Word16 phase_mean_mean, phase_mean_std; 
    Word32 Acc; 
    Word16 flag; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (5 * SIZE_Word16 +  1 * SIZE_Word32), "dummy");
#endif
    flag = 0; move16(); 
    if(L_sub(ipd_reg_num_sm, 12800) > 0) flag = add(flag,1);/* 50 q8 */ 

    Acc = L_mult(3277, phase_mean_buf[0]); 
    FOR(i = 1; i < IPD_FNUM; i ++) 
    { 
        Acc = L_mac(Acc, 3277, phase_mean_buf[i]); 
    } 
    phase_mean_mean = round_fx(Acc); 
    phase_mean_std = abs_s(sub(phase_mean_mean, mean_ipd)); 

    IF(*f_num == 0) 
    { 
        *phase_mean_std_sm = phase_mean_std; move16(); 
        *f_num = add(1, *f_num); move16(); 
    } 
    ELSE 
    { 
        Acc = L_mult(*phase_mean_std_sm, 32256); 
        Acc = L_mac(Acc, phase_mean_std, 512); 
        *phase_mean_std_sm = round_fx(Acc); move16(); 
    } 
    if(sub(*phase_mean_std_sm, 328)< 0) flag = add(flag,1); /* 0.08 */ 
    Acc = L_mult(*ipd_mean_sm, 32256); 
    Acc = L_mac(Acc, mean_ipd, 512); 
    mean_ipd = round_fx(Acc); 
    Ipd = 0; move16(); 
    IF (sub(abs_s(en_ratio_sm), 4864) <= 0) /* 9.5 */ 
    { 
        if(flag !=0) Ipd = extract_l(mean_ipd); 
    } 

    *ipd_mean_sm = mean_ipd; move16(); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (Ipd); 
}   

/*************************************************************************
* updateIpd
*
* Smooth IPD in order to avoid fast change from PI to -PI
**************************************************************************/
static void updateIpd(Word32 *mean_ipd, Word16 *pre_ipd_mean, Word16 *ipd_num, 
                      Word16 *ipd_reg_num, Word16 *ipd_reg_num_sm, 
                      Word16 *phase_mean_buf)
{
    Word32 tmp32;
    Word16 tmp16;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 + 1 * SIZE_Word32), "dummy");
#endif
    tmp32 = L_msu0(*mean_ipd, *pre_ipd_mean, 1);
    IF(L_sub(L_abs(tmp32), PI_1D5_4096) > 0)
    {
        IF(sub(*ipd_num, 10) < 0)
        {
            *mean_ipd = L_mult0(1, *pre_ipd_mean);
            *ipd_num = add(1, *ipd_num);
        }
        ELSE
        {
            *ipd_num = 0; move16();
            *pre_ipd_mean = extract_l(*mean_ipd); move16();
        }
    }
    ELSE
    {
        *pre_ipd_mean = extract_l(*mean_ipd); move16();
        *ipd_num = 0; move16();
    }
    IF(L_sub(L_abs(*mean_ipd), 10240) > 0) 
    {
        *ipd_reg_num = add(1, *ipd_reg_num);
        *ipd_reg_num = s_min(*ipd_reg_num, 70);
    }
    ELSE
    {
        *ipd_reg_num = sub(*ipd_reg_num, 1);
        *ipd_reg_num = s_max(*ipd_reg_num, 0);
    }

    tmp16 = shr(sub(*ipd_reg_num_sm, shl(*ipd_reg_num, 8)), 6);
    *ipd_reg_num_sm = sub(*ipd_reg_num_sm, tmp16);
    *phase_mean_buf = extract_l(*mean_ipd); move16();

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* excludeOutlier
*
* exclude the IPD outlier
**************************************************************************/
static Word32 excludeOutlier(Word32 mean_ipd, Word16 *temp_phase_ipd, Word16 pre_Ipd)
{
    Word32 tmp32, sum_ipd;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  2 * SIZE_Word32), "dummy");
#endif
    IF (mean_ipd > 0)
    {
        sum_ipd = 0; move32();
        FOR(i = STARTBANDPHA; i< STARTBANDPHA+BANDPHA; i++)
        {
            tmp32 = L_abs(L_msu0(mean_ipd, 1, temp_phase_ipd[i]));
            test();
            IF ((temp_phase_ipd[i] < 0) && (L_sub(tmp32, PI_4096_D_4) > 0))
            {
                temp_phase_ipd[i] = add(temp_phase_ipd[i], PI_2_4096); move16();
            }
            sum_ipd = L_mac0(sum_ipd, 1, temp_phase_ipd[i]);
        }
        mean_ipd = L_mls(sum_ipd, 5461); /* 1/6 */
    }
    ELSE
    {
        tmp32 = L_abs(L_msu0(mean_ipd, 1, temp_phase_ipd[STARTBANDPHA]));
        test();
        IF ((temp_phase_ipd[STARTBANDPHA] > 0) && (L_sub(tmp32, PI_4096_D_4) > 0))
        {
            temp_phase_ipd[STARTBANDPHA] = sub(temp_phase_ipd[STARTBANDPHA], PI_2_4096); move16();
        }
        sum_ipd = L_mac0(0, 1, temp_phase_ipd[STARTBANDPHA]);
        FOR(i = STARTBANDPHA + 1; i< STARTBANDPHA + BANDPHA; i++)
        {
            tmp32 = L_abs(L_msu0(mean_ipd, 1, temp_phase_ipd[i]));
            test();
            IF ((temp_phase_ipd[i] > 0) && (L_sub(tmp32, PI_4096_D_4) > 0))
            {
                temp_phase_ipd[i] = sub(temp_phase_ipd[i], PI_2_4096); move16();
            }
            sum_ipd = L_mac0(sum_ipd, 1, temp_phase_ipd[i]);
        }
        mean_ipd = L_mls(sum_ipd, 5461); /* 1/6 */
    }
    test();
    if ((sub(pre_Ipd, PI_08_4096) > 0) && (L_add(mean_ipd, PI_08_4096) < 0))
    {
        mean_ipd = L_add(mean_ipd, PI_2_4096);
    }
    test();
    if ((add(pre_Ipd, PI_08_4096) < 0) && (L_sub(mean_ipd, PI_08_4096) > 0))
    {
        mean_ipd = L_sub(mean_ipd, PI_2_4096);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(mean_ipd);
}

/*************************************************************************
* getGlobalPhaseStd
*
* compute the standard deviation of IPD over several subband
**************************************************************************/
static Word16 getGlobalPhaseStd(Word32 mean_ipd, Word16 *temp_phase_ipd, 
                                Word32 *energy_bin_sm)
{
    Word16 i, tmp16_2, tmp16,  std_ipd;
    Word32 tmp32_2,tmp32;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  2 * SIZE_Word32), "dummy");
#endif
    tmp16_2 = extract_l(mean_ipd);
    tmp16 = sub(temp_phase_ipd[STARTBANDPHA], tmp16_2);
    tmp32 = L_mls(energy_bin_sm[STARTBANDPHA], tmp16);
    tmp16 = round_fx(tmp32);
    tmp32_2 = L_mult(tmp16, tmp16);
    FOR(i = STARTBANDPHA + 1; i< STARTBANDPHA + BANDPHA; i++)
    {
        tmp16 = sub(temp_phase_ipd[i], tmp16_2);
        tmp32 = L_mls(energy_bin_sm[i], tmp16);
        tmp16 = round_fx(tmp32);
        tmp32_2 = L_mac(tmp32_2, tmp16, tmp16);
    }

    std_ipd = L_sqrt(tmp32_2);

    move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(std_ipd );
}

/*************************************************************************
* getWeightedMeanGlobalPhase
*
* compute the whole wideband IPD based on energy weighted subband IPD
**************************************************************************/
static Word32 getWeightedMeanGlobalPhase(Word32 *energy_bin_sm, Word16 *temp_phase_ipd)
{
    Word16 i;
    Word32 mean_ipd; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32), "dummy");
#endif
    mean_ipd = L_mls(L_shr(energy_bin_sm[STARTBANDPHA], 3), temp_phase_ipd[STARTBANDPHA]);
    FOR(i = STARTBANDPHA + 1; i< STARTBANDPHA + BANDPHA; i++)
    {
        mean_ipd = L_add(mean_ipd, L_mls(L_shr(energy_bin_sm[i], 3), temp_phase_ipd[i]));/* Q28 */
    }
    mean_ipd = L_shr(mean_ipd, 13);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(mean_ipd); 
}

/*************************************************************************
* selectITDmain
*
* select the final ITD based on positive and negative ITD
**************************************************************************/
static Word16 selectITDmain(Word16 nb_pos, Word16 nb_neg, Word32 std_itd_pos, 
                            Word32 std_itd_neg, Word16 mean_itd_pos, Word16 mean_itd_neg, 
                            Word16 pre_Itd)
{
    Word16 Itd, tmp16;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16), "dummy");
#endif
    IF (sub(nb_pos, nb_neg) > 0)
    {
        test();
        IF ((L_sub(std_itd_pos, std_itd_neg) < 0)||(sub(nb_pos, shl(nb_neg, 1)) >= 0))
        {
            tmp16 = add(abs_s(mean_itd_pos), 128);
            Itd = shr(tmp16, 8);
        }
        ELSE
        {
            IF (L_sub(L_shl(std_itd_neg, 1), std_itd_pos) < 0)
            {

                tmp16 = add(128, abs_s(mean_itd_neg));
                Itd = negate(shr(tmp16, 8));
            }
            ELSE
            {
                IF (pre_Itd > 0)
                {
                    tmp16 = add(abs_s(mean_itd_pos), 128);
                    Itd = shr(tmp16, 8);
                }
                ELSE
                {
                    tmp16 = add(128, abs_s(mean_itd_neg));
                    Itd = negate(shr(tmp16, 8));
                }
            }
        }
    }
    ELSE
    {
        test();
        IF ((L_sub(std_itd_neg, std_itd_pos) < 0)||(sub(nb_neg, shl(nb_pos, 1)) >= 0))
        {
            tmp16 = add(128, abs_s(mean_itd_neg));
            Itd = negate(shr(tmp16, 8));
        }
        ELSE
        {
            IF (L_sub(L_shl(std_itd_pos, 1), std_itd_neg)< 0)
            {
                tmp16 = add(abs_s(mean_itd_pos), 128);
                Itd = shr(tmp16, 8);
            }
            ELSE
            {
                IF (pre_Itd > 0)
                {
                    tmp16 = add(abs_s(mean_itd_pos), 128);
                    Itd = shr(tmp16, 8);
                }
                ELSE
                {
                    tmp16 = add(128, abs_s(mean_itd_neg));
                    Itd = negate(shr(tmp16, 8));
                }
            }
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(Itd);
}

/*************************************************************************
* checkMeaningfulITD
*
* Check if it is a meaningful ITD
**************************************************************************/
static Word16 checkMeaningfulITD(Word32 std_itd_sm, Word32 nb_idx_sm, 
                                 Word16 pre_Itd, Word16 flagITDstd)
{

    Word16 flag;
    Word32 Ltmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32), "dummy");
#endif
    flag = flagITDstd; move16();
    IF(flagITDstd ==0) 
    {
        if(L_sub(std_itd_sm, 768) >= 0) flag = add(flag,1);
        Ltmp = L_deposit_l(2048);
        if(pre_Itd !=0) Ltmp= L_sub(Ltmp, 256);
        if(L_sub(nb_idx_sm, Ltmp) <= 0) flag = add(flag,1);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(flag);
}

/*************************************************************************
* smoothMeanITD
*
* smoothing of the whole wideband ITD value over time
**************************************************************************/
static Word16 smoothMeanITD(Word16 mean_itd, Word16 *pre_itd)
{
    Word32 Ltemp;
    Word16 tmp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32), "dummy");
#endif
    Ltemp = L_mult0(*pre_itd, 63);
    Ltemp = L_shr(L_mac0(Ltemp, mean_itd, 1), 6);

    tmp = extract_l(Ltemp);
    *pre_itd = tmp; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(tmp);
}

/*************************************************************************
* smoothITD
*
* selection between weak and strong smoothing based whole wideband ITD 
**************************************************************************/
static void smoothITD(Word16 nb_inst, Word16 std_itd_inst,  Word16 *std_itd, 
                      Word16 flagHighEner, Word16 *nb, Word16 *mean_itd, Word16 mean_itd_inst, 
                      Word32 *Crxyt, Word32 *Crxyt2, Word32 *Cixyt, Word32 *Cixyt2,
                      Word16 *std_itd_sm, Word16 *nb_idx_sm)
{
    Word16 tmp16;
    Word16 i;
    Word16 flag;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  0 * SIZE_Word32), "dummy");
#endif
    flag = 0; move16();
    IF(flagHighEner !=0) 
    {
        if(sub(nb_inst, BANDITD) == 0)  flag = add(flag,1);
        if (L_sub(std_itd_inst, 512) < 0) flag = add(flag,1);
        flag = shr(flag,1);
        if (nb_inst == 0) flag = add(flag,1);
    }

    IF(flag != 0) 
    {
        *std_itd_sm = std_itd_inst; move16();
        *std_itd    = std_itd_inst; move16();
        *nb         = nb_inst; move16();
        *nb_idx_sm  = shl(nb_inst, 8);
        *mean_itd   = mean_itd_inst; move16();
        FOR(i=STARTBANDITD; i<STARTBANDITD+BANDITD; i++)
        {
            Crxyt[i] = Crxyt2[i]; move32();
            Cixyt[i] = Cixyt2[i]; move32();
        }
    }
    ELSE
    {
        tmp16 = shr(sub(*std_itd_sm, *std_itd), 6);
        *std_itd_sm = sub(*std_itd_sm, tmp16);

        tmp16 = shr(sub(*nb_idx_sm, shl(*nb, 8)), 6);
        *nb_idx_sm = sub(*nb_idx_sm, tmp16);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calcStandITD
*
* compute standard deviation of positive and negative ITD
**************************************************************************/
static void calcStandITD(Word16 *temp_phase, Word16 *itd, 
                         Word16 nb_pos, Word16 mean_itd_pos, Word16 *std_itd_pos2, 
                         Word16 nb_neg, Word16 mean_itd_neg, Word16 *std_itd_neg2)
{
    Word32 std_itd_pos, std_itd_neg;
    Word16 i, tmp16, *ptr0, *ptr1;
    const Word16 *ptr_pos, *ptr_neg;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 +  2 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    ptr0 = temp_phase;
    ptr1 = itd;
    /* standard deviation for positive and negative mean ITD with low and high smoothing */
    std_itd_pos = 0; move32();
    std_itd_neg = 0; move32();
    ptr_pos = INV_nb_idx;
    ptr_neg = INV_nb_idx;

    FOR(i = 0; i< BANDITD; i++)
    {
        IF (*ptr0 > 0)
        {
            tmp16 = sub(*ptr1, mean_itd_pos);
            std_itd_pos = L_mac(std_itd_pos, tmp16, tmp16);
            ptr_pos++;
        }
        ELSE
        {
            tmp16 = sub(*ptr1, mean_itd_neg);
            std_itd_neg = L_mac(std_itd_neg, tmp16, tmp16);
            ptr_neg++;
        }
        ptr0++; ptr1++;
    }

    *std_itd_pos2 = getITDstd(std_itd_pos, (Word16)*ptr_pos, nb_pos);
    *std_itd_neg2 = getITDstd(std_itd_neg, (Word16)*ptr_neg, nb_neg);
    move32(); move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* getITDstd
*
* Compute of square root for standard deviation
**************************************************************************/
static Word16 getITDstd(Word32 std_itd, Word16 n, Word16 nb)
{
    Word16 tmp16;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32), "dummy");
#endif
    tmp16 = 1792; move16();/* 7*256 */
    IF (nb > 0)
    {
        tmp16 = L_sqrt(L_mls(std_itd, n));
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (tmp16);
}

/*************************************************************************
* updateMemEner
*
* smoothing of energy
**************************************************************************/
static Word32 updateMemEner(Word32 mem_energy, Word32 energy)
{
    Word32 tmp32;
    Word16 tmp16; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32), "dummy");
#endif
    tmp16 = L_sqrt(energy);
    tmp16 = shl(tmp16, 2);

    tmp32 = L_shr(L_msu0(mem_energy, 1, tmp16), 2);
    mem_energy = L_sub(mem_energy, tmp32);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(mem_energy);
}

/*************************************************************************
* udpdateEnerBin
*
* smooth the energy of each bin
**************************************************************************/
static void udpdateEnerBin(Word16 f_num, Word16 n, Word32 *energy_bin, 
                           Word32 *energy_bin_sm)
{
    Word32 *ptr0, *ptr1, tmp32;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif
    ptr1 = energy_bin_sm;
    ptr0 = energy_bin;
    IF( f_num == 0)
    {
        FOR(i = 0; i < n; i++)
        {
            *ptr1++ = *ptr0++; move32();
        }
    }
    ELSE
    {
        FOR(i = 0; i < n; i++)
        {
            tmp32 = L_shr(L_sub(*ptr1, *ptr0), 6);
            *ptr1 = L_sub(*ptr1, tmp32); move32();
            ptr1++; ptr0++;
        }
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* udpdateEner_ratio_sm
*
* update the energy ratio between two channels
**************************************************************************/
static void udpdateEner_ratio_sm(Word32 en_l, Word32 en_r, Word16 *en_ratio_sm) 
{ 
    Word32 tmp32, tmp32_2; 
    Word16 tmp16; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  2 * SIZE_Word32), "dummy");
#endif        
    tmp32 = L_add(en_l, 1); 
    tmp32_2 = L_add(en_r, 1); 
    tmp16 = ild_calculation(tmp32, tmp32_2, 0, 0);/* q9 */ 
    tmp32 = L_mult(tmp16 , 512); 
    tmp32 = L_mac(tmp32, *en_ratio_sm, 32256); 
    *en_ratio_sm = round_fx(tmp32); move16(); 
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return; 
} 

/*************************************************************************
* normalizeEnerBin
*
* Normalize the energy of each bin
**************************************************************************/
static void normalizeEnerBin(Word16 n, Word32 en_l, Word32 en_r, Word32 *energy_bin)
{
    Word32 tmp32, *ptrL;
    Word16 i, tmp16_3, tmp16_2, tmp16; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  1 * SIZE_Word32 + 1 * SIZE_Ptr), "dummy");
#endif
    tmp32 = L_add(en_l, en_r);
    tmp32 = L_add(1, tmp32);
    tmp16_3 = norm_l(tmp32);
    tmp32 = L_shl(tmp32, tmp16_3);
    tmp16 = L_Extract_lc(tmp32, &tmp16_2);
    ptrL = energy_bin;
    FOR(i = 0; i < n; i++)
    {
        tmp32 = Div_32(*ptrL, tmp16_2, tmp16);
        *ptrL++ = L_shl(tmp32, tmp16_3); move32();/* Q31 */
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* Exp16Array_stereo
*
* Find number of shift to normalize a 16-bit array variable
**************************************************************************/
Word16 Exp16Array_stereo(Word16 n, Word16 *s_real, Word16 *s_imag)
{ 
    Word16 sMax, sAbs, k, exp; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    sMax = s_max( abs_s( s_imag[0] ), abs_s( s_real[0] )); 
    FOR ( k = 1; k < n; k++ ) 
    { 
        sAbs = abs_s( s_real[k] ); 
        sMax = s_max( sMax, sAbs ); 
        sAbs = abs_s( s_imag[k] ); 
        sMax = s_max( sMax, sAbs ); 
    } 
    exp = norm_s( sMax ); 
    if(sMax == 0)
    {
        exp = 15; move16();
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(exp);
} 

/*************************************************************************
* calcEnerITD
*
* compute weak and strong smoothing cross spectrum and subband energy
**************************************************************************/
static void calcEnerITD(Word16 n, Word32 *energyL, Word32 *energyR, 
                        Word32 *energy_bin, Word16 q_left, Word16 q_right, 
                        Word32 *Crxyt, Word32 *Cixyt, Word32 *Crxyt2, Word32 *Cixyt2,
                        Word16 *temp_phase, Word16 *temp_phase_ipd, 
                        Word16 *L_real, Word16 *L_imag, Word16 *R_real, Word16 *R_imag)
{ 
    Word16 i; 
    Word16  tmpLr, tmpLi, tmpRr, tmpRi; 
    Word16 *ptrLr, *ptrLi, *ptrRr, *ptrRi; 
    Word16 *ptr_phase, *ptr_phase_ipd; 
    Word32 *ptr_enerBin, *ptr_Crxyt, *ptr_Cixyt, *ptr_Crxyt2, *ptr_Cixyt2; 
    Word32 tmp32, tmp32_2, temp, tempi; 
    Word16 normL, normR;
    Word16 normL2, normR2, normLR; 
    Word32 energyL_loc, energyR_loc; 
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (8 * SIZE_Word16 +  6 * SIZE_Word32 + 11 * SIZE_Ptr), "dummy");
#endif
    normL = Exp16Array_stereo(n, L_real, L_imag); 
    normR = Exp16Array_stereo(n, R_real, R_imag); 
    normL = sub(normL,1); 
    normR = sub(normR,1); 
    energyL_loc = L_deposit_l(0); 
    energyR_loc = L_deposit_l(0); 

    normL2 = shl(add(q_left, normL),1); 
    normR2 = shl(add(q_right, normR),1); 
    normLR = add(add(normL, normR),add(q_right, q_left)); 
    ptrLr         = L_real; 
    ptrLi         = L_imag; 
    ptrRr         = R_real; 
    ptrRi         = R_imag; 
    ptr_Crxyt     = Crxyt; 
    ptr_Cixyt     = Cixyt; 
    ptr_Crxyt2    = Crxyt2; 
    ptr_Cixyt2    = Cixyt2; 
    ptr_enerBin   = energy_bin; 
    ptr_phase     = temp_phase; 
    ptr_phase_ipd = temp_phase_ipd; 
    FOR(i=0; i<n; i++) 
    { 
        tmpLr       = shl(*ptrLr++, normL); 
        tmpLi       = shl(*ptrLi++, normL); 
        tmpRr       = shl(*ptrRr++, normR); 
        tmpRi       = shl(*ptrRi++, normR); 
        tmp32       = L_mult0(tmpLr, tmpLr); 
        tmp32       = L_mac0(tmp32, tmpLi, tmpLi); 
        energyL_loc = L_add(energyL_loc, L_shr(tmp32,1)); 
        tmp32_2     = L_mult0(tmpRr, tmpRr); 
        tmp32_2     = L_mac0(tmp32_2, tmpRi, tmpRi); 
        energyR_loc = L_add(energyR_loc, L_shr(tmp32_2,1)); 
        tmp32       = L_shr(tmp32, normL2); 
        tmp32_2     = L_shr(tmp32_2, normR2); 
        *ptr_enerBin++ = L_add(tmp32, tmp32_2); move32(); 
        temp        = L_mult0(tmpLr, tmpRr); 
        temp        = L_mac0(temp, tmpLi, tmpRi); 
        tempi       = L_mult0(tmpLi, tmpRr); 
        tempi       = L_msu0(tempi, tmpLr, tmpRi); 
        temp        = L_shr(temp , normLR); 
        tempi       = L_shr(tempi , normLR); 
        tmp32       = L_shr(L_sub(*ptr_Crxyt, temp), 6); 
        *ptr_Crxyt  = L_sub(*ptr_Crxyt, tmp32); move32(); 
        tmp32       = L_shr(L_sub(*ptr_Cixyt, tempi), 6); 
        *ptr_Cixyt  = L_sub(*ptr_Cixyt, tmp32); move32(); 
        tmp32       = L_shr(L_sub(*ptr_Crxyt2, temp), 2); 
        *ptr_Crxyt2 = L_sub(*ptr_Crxyt2, tmp32); move32(); 
        tmp32       = L_shr(L_sub(*ptr_Cixyt2, tempi), 2); 
        *ptr_Cixyt2 = L_sub(*ptr_Cixyt2, tmp32); move32(); 
        *ptr_phase++     = negate(arctan2_fix32(*ptr_Cixyt,*ptr_Crxyt)); move16(); 
        *ptr_phase_ipd++ = arctan2_fix32(*ptr_Cixyt2,*ptr_Crxyt2); move16(); 
        ptr_Crxyt++; ptr_Cixyt++; ptr_Crxyt2++; ptr_Cixyt2++; 
    } 
    energyL_loc  = L_shr(energyL_loc, normL2-1); 
    energyR_loc  = L_shr(energyR_loc, normR2-1); 
    *energyL = L_add(energyL_loc, *energyL); move32();
    *energyR = L_add(energyR_loc, *energyR); move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return; 
}

/*************************************************************************
* calcMeanITD
*
* Calculate the mean of the ITDs
**************************************************************************/
static void calcMeanITD(Word16 *temp_phase, Word16 *itd, 
                        Word16 *mean_itd_pos, Word16 *mean_itd_neg,
                        Word16 *nb_pos2, Word16 *nb_neg2)
{
    Word16 i, nb_pos, nb_neg;
    Word32 sum_itd_pos, sum_itd_neg;
    const Word16 *ptr1, *ptr2;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  2 * SIZE_Word32 + 2 * SIZE_Ptr), "dummy");
#endif
    ptr1 = NFFT_D_2_PI_I;
    ptr2 = MAX_PHASE_I;

    sum_itd_pos = 0; move32();
    sum_itd_neg = 0; move32();
    nb_pos = 0; move16();
    nb_neg = 0; move16();
    FOR(i = STARTBANDITD; i< STARTBANDITD+BANDITD; i++)
    {
        itd[i] = mult_r(temp_phase[i], *ptr1++); move16();
        IF(sub(abs_s(temp_phase[i]), *ptr2++) < 0)
        {
            IF (temp_phase[i] > 0)
            {
                sum_itd_pos = L_mac(sum_itd_pos, 16384, itd[i]);
                nb_pos = add(1, nb_pos);
            }
            ELSE
            {
                sum_itd_neg = L_mac(sum_itd_neg, 16384, itd[i]);
                nb_neg = add(1, nb_neg);
            }
        }
    }
    *mean_itd_neg = getMeanITD(sum_itd_neg, nb_neg); move16();
    *mean_itd_pos = getMeanITD(sum_itd_pos, nb_pos); move16();
    *nb_pos2 = nb_pos; move16();
    *nb_neg2 = nb_neg; move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* getMeanITD
*
* compute the mean ITD from sum of subband ITD
**************************************************************************/
static Word16 getMeanITD(Word32 sum_itd, Word16 nb)
{
    Word16 mean;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16), "dummy");
#endif
    mean = 0; move16();
    IF(nb > 0)
    {
        mean = round_fx(L_shl(sum_itd,1));
        mean = mult(mean, INV_nb_idx[nb]);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return (mean);
}


#endif /*LAYER_STEREO*/
