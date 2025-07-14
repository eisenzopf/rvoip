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
#include "oper_32b.h"
#include "fft.h"

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
#endif
extern Word16 cosw[80],sinw[80],sinw_p1[80],sinw_s1[80];
extern Word16 pidx_IFT[80],pidx_FT[80];

#define CFFT5_R1   (Word16)((32768+ 2*C_fx51 + C_fx52)>>1)
#define CFFT5_R2   (Word16)((32768+ 2*C_fx51 - C_fx52)>>1)
#define CFFT5_I1   (Word16) (C_fx53>>1)
#define CFFT5_I2   (Word16)((2*C_fx54 - C_fx53)>>1)
#define CFFT5_I3   (Word16)((C_fx53 + C_fx55)>>1)

static void fft_5_format_2(Word32 *pRe, Word32 *pIm, Word16 *psRe, Word16 *psIm, Word16 QVal);
static Word32 calc_R0 (Word16 ps0, Word32 *pL, Word16 ar14, Word16 ar32, Word16 QVal);
static Word32 calcR4  (Word32 L_temp0, Word16 arx, Word16 ary);
static Word32 calcI4  (const Word16 cx, const Word16 cy, Word16 six, Word16 siy);
static void   calcRI4 (Word32 L_temp0, Word16 arx, Word16 ary, const Word16 cx, 
                       const Word16 cy, Word16 six, Word16 siy, Word32 *L_a, 
                       Word32 *L_s, Word16 QVal);
static void calc_fft5R(Word16 ps0, Word32 *pL, Word16 ar14, Word16 si14, 
                       Word16 ar32, Word16 si32, Word16 QVal);
static void calc_fft5I(Word16 ps0, Word32 *pL, Word16 ar14, Word16 si14, 
                       Word16 ar32, Word16 si32, Word16 QVal);
static void fft16R    (Word32 *sumReIm4, Word32 *sumReIm, Word32 *Re);
static void fft16I    (Word32 *sumReIm4, Word32 *sumReIm, Word32 *Im);
static void calcSum2  (Word32 *Re, Word32 *sumRe);
static void calcSum4RI(Word32 *ReIm, Word32 *sumReIm);

static void calc_RiFFTx   (Word16 *x, Word16 *sRe, Word16 *sIm, Word16 norm_shift);
static void calc_RiFFT_1_4(Word16 *x0, Word16 *x0f, Word16 norm_shift, 
                           Word16 sinw_s1, Word16 sinw_p1, Word16 cosw, 
                           Word16 *Re, Word16 *Ref, Word16 *Im, Word16 *Imf);
static void calc_RiFFT_1_2(Word16 *x0, Word16 *x0f, Word16 norm_shift, 
                           Word16 sinw_s1, Word16 sinw_p1, Word16 cosw, 
                           Word16 *Ref, Word16 *Imf);

static void s_calcSum2(Word32 *Re, Word16 *sumRe, Word16 QVal);
static void s_calcSum4RI(Word16 *ReIm, Word16 *sumReIm);
static void s_fft16R(Word16 *sumReIm4, Word16 *sumReIm, Word32 *Re, Word16 shift);
static void s_fft16I(Word16 *sumReIm4, Word16 *sumReIm, Word32 *Im, Word16 shift);
static void s_fft16Ri(Word16 *sumReIm4, Word16 *sumReIm, Word16 shift, Word16 *x);
static void s_fft16Ii(Word16 *sumReIm4, Word16 *sumReIm, Word16 shift, Word16 *x);

static Word32 axplusby0(Word16 x, Word16 y, const Word16 ca, const Word16 cb);
static Word32 axpmy_bzpmt0(Word16 x, Word16 y, const Word16 cax, const Word16 cay,
                           Word16 z, Word16 t, const Word16 cbz, const Word16 cbt);
static void fft_16x_format_2(Word32 *ReIm, Word16 QVal);
static void fft_16x_format_2i(Word32 *ReIm, Word16 QVal, Word16 *x, Word16 x_q);
static void endRfft(Word32 *ReIm, Word16 *x, Word16 Qfft16);
static void subEndRfft(Word32 *ptrReIm, Word32 *ptrReImf, Word16 *ptr0, Word16 *ptr0f, 
                       Word16 Qfft16, Word16 Qff16_1, Word16 *ptr_cos, Word16 *ptr_sin, 
                       Word16 n);
static void twiddleReIm(Word32 *pLRe, Word32 *pLIm, Word32 *pRe, Word32 *pIm, 
                        Word16 *ptr_twiddleRe, Word16 *ptr_twiddleIm,
                        Word16 Qfft5, Word16 Qfft51);

/*************************************************************************
* fixDoRFFTx
*
* Do 160-points real FFT
**************************************************************************/
void fixDoRFFTx(Word16 x[], Word16 *x_q)
{   
    Word16 i;
    Word16 dataNo,groupNo;
    Word32 ReIm2[160],*Re, *Im;
    Word32 *pRe,*pIm;
    Word32 *pRe0,*pIm0;
    Word32 *pLRe,*pLIm;
    Word16 s_Re[80], s_Im[80], *psRe, *psIm, QVal; 
    Word16 *ptrRe, *ptrIm;
    Word16 *ptr_x;
    Word16 Qfft16, Qfft5, Qfft51;
    Word32 ReIm[5*32], *ptrReIm;
    Word16 *ptr_twiddleRe, *ptr_twiddleIm;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((7 + 2 * 80) * SIZE_Word16 +  (2 * 160) * SIZE_Word32 + 16 * SIZE_Ptr), "dummy");
#endif
    *x_q  = Exp16Array(160,x);
    Re    = ReIm2;
    Im    = ReIm2+80;
    QVal  = sub(*x_q,1);
    ptr_x = x;
    ptrRe = s_Re;
    ptrIm = s_Im;
    FOR (dataNo=0; dataNo<5; dataNo++)
    {   
        psRe = ptrRe++;
        psIm = ptrIm++;
        FOR (groupNo=0; groupNo<16; groupNo++)
        { 
            *psRe = shl(*ptr_x++, QVal); move16();
            *psIm = shl(*ptr_x++, QVal); move16();
            psRe += 5;
            psIm += 5;
        }
    }

    pRe  = Re ;
    pIm  = Im ;
    psRe = s_Re;
    psIm = s_Im;
    FOR (groupNo=0; groupNo<16; groupNo++)
    {  
        fft_5_format_2(pRe, pIm, psRe, psIm, (Word16)13);   
        pRe += 5;
        pIm += 5;
        psRe += 5;
        psIm += 5;
    }
    Qfft5  = Exp32Array(160,ReIm2);
    Qfft5  = sub(Qfft5, 1);
    Qfft51 = sub(Qfft5, 1);

    ptrReIm = ReIm;
    pRe0 = ptrReIm;
    pIm0 = ptrReIm+16;
    pRe  = Re ;
    pIm  = Im ;
    FOR(i=0; i<16; i++)
    {
        *pRe0++ = *pRe; move32();
        *pIm0++ = *pIm; move32();
        pRe += 5;
        pIm += 5;
    }

    fft_16x_format_2(ptrReIm, Qfft5);
    QVal = sub(Qfft51,3);
    ptrReIm += 32;
    pRe0 = Re + 1;
    pIm0 = Im + 1;

    ptr_twiddleRe = twiddleRe;
    ptr_twiddleIm = twiddleIm;

    FOR (dataNo=1; dataNo<5; dataNo++)
    {   
        pRe = pRe0++;
        pIm = pIm0++;
        pLRe = ptrReIm;
        pLIm = ptrReIm+16;
        *pLRe++ = *pRe; move32();
        *pLIm++ = *pIm; move32();

        twiddleReIm(pLRe, pLIm, pRe, pIm, ptr_twiddleRe+1, ptr_twiddleIm+1,
                    Qfft5, Qfft51);
        ptr_twiddleRe += 16;
        ptr_twiddleIm += 16;
        fft_16x_format_2(ptrReIm, Qfft5);
        ptrReIm += 32;
    }

    Qfft16 = Exp32Array(160,ReIm);
    Qfft16 = sub(Qfft16,2);

    *x_q = add(*x_q,Qfft16);

    endRfft(ReIm, x, Qfft16);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* twiddleReIm
*
* Twiddle factor calculation for mix-factor fft
**************************************************************************/
static void twiddleReIm(Word32 *pLRe, Word32 *pLIm, Word32 *pRe, Word32 *pIm, 
                        Word16 *ptr_twiddleRe, Word16 *ptr_twiddleIm,
                        Word16 Qfft5, Word16 Qfft51)
{
    Word16 tmp, tmp1;
    Word32 L_tmp, L_tmp1;
    Word16 blockNo;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    FOR(blockNo=1;blockNo<16;blockNo++)
    {   
        pRe += 5;
        pIm += 5;

        tmp  = round_fx(L_shl(*pRe, Qfft5));
        tmp1 = round_fx(L_shl(*pIm, Qfft5));

        L_tmp   = L_mult0(tmp, *ptr_twiddleRe);
        L_tmp   = L_msu0(L_tmp, tmp1, *ptr_twiddleIm );
        L_tmp   = L_shr(L_tmp, Qfft51);
        *pLRe++ = L_tmp; move32();

        L_tmp1  = L_mult0(tmp1, *ptr_twiddleRe);
        L_tmp1  = L_mac0(L_tmp1, tmp, *ptr_twiddleIm);
        L_tmp1  = L_shr(L_tmp1, Qfft51);
        *pLIm++ = L_tmp1; move32();
        ptr_twiddleRe++;
        ptr_twiddleIm++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* endRfft
* subroutine called at the end of fixDoRFFTx
* to compute the real and imaginary part of 80 FFT coefficients
* (by groups of 8 coefficients)
**************************************************************************/
static void endRfft(Word32 *ReIm, Word16 *x, Word16 Qfft16)
{
    Word16 dataNo;
    Word32 *pRe0,*pIm0, *pRef,*pImf;
    Word16 *ptr0, *ptr0f, *ptr1, *ptr1f;
    Word16 s_sRe, s_aIm;
    Word16 *ptr_cos, *ptr_sin;
    Word32 *ptrReIm, *ptrReImf;
    Word32 L_temp0, L_temp1, La, Ls, L_temp;
    Word16 Qfft16_1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16 +  5 * SIZE_Word32 + 12 * SIZE_Ptr), "dummy");
#endif
    Qfft16_1 = sub(Qfft16,1);
    ptr0  = x;
    ptr0f = ptr0 +80;
    ptr1  = ptr0f+1;
    ptr1f = ptr1+80;

    /* data0 */
    ptrReIm = ReIm; 
    pRe0 = ptrReIm; 
    pIm0 = ptrReIm+16;

    L_temp0 = L_add(*pRe0, *pIm0);
    L_temp1 = L_sub(*pRe0, *pIm0);
    *ptr0  = round_fx(L_shl(L_temp0,Qfft16)); move16();
    *ptr0f = round_fx(L_shl(L_temp1,Qfft16)); move16();
    pRe0++; 
    pIm0++;
    *ptr1 = *ptr1f = 0; move16(); move16();

    ptr_cos = cosw;
    ptr_sin = sinw;
    subEndRfft(ptrReIm+1, ptrReIm+31, ptr0+5, ptr0f-5, Qfft16, Qfft16_1, ptr_cos+5, ptr_sin+5, 7);
    pRe0 += 7;
    pIm0 = pRe0 + 16;
    pImf = ReIm + 24;
    pRef = pImf - 16;

    ptr_cos = cosw + 40;
    ptr_sin = sinw + 40;
    La = L_add(*pRe0, *pRef);
    Ls = L_sub(*pIm0, *pImf);
    s_sRe    = round_fx(L_shl(L_sub(*pRe0, *pRef),Qfft16));
    s_aIm    = round_fx(L_shl(L_add(*pIm0, *pImf),Qfft16));

    L_temp   = L_mult(*ptr_sin, s_sRe);
    L_temp   = L_shr(L_temp,Qfft16);
    L_temp0  = L_sub(La, L_temp);
    ptr0[40] = round_fx(L_shl(L_temp0,Qfft16_1)); move16();

    L_temp   = L_mult(*ptr_sin, s_aIm);
    L_temp   = L_shr(L_temp,Qfft16);
    L_temp0  = L_sub(Ls, L_temp);
    ptr0[121] = round_fx(L_shl(L_temp0,Qfft16_1));  move16();

    /* datax */
    ptrReImf= ReIm+159;
    ptr_cos = cosw;
    ptr_sin = sinw;
    FOR (dataNo=1; dataNo<5; dataNo++)
    {
        ptrReIm += 32; 
        ptr0++;
        ptr0f--;
        ptr_cos++;
        ptr_sin++;
        subEndRfft(ptrReIm, ptrReImf, ptr0, ptr0f, Qfft16, Qfft16_1, ptr_cos, ptr_sin, 8);
        ptrReImf -= 32;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* subEndRfft
* subroutine called  by endRfft
* to compute the real and imaginary part of n (=7 or 8) FFT coefficients
* with windowing
**************************************************************************/
static void subEndRfft(Word32 *ptrReIm, Word32 *ptrReImf, Word16 *ptr0, Word16 *ptr0f, 
                       Word16 Qfft16, Word16 Qfft16_1, Word16 *ptr_cos, Word16 *ptr_sin, 
                       Word16 n)
{
    Word16 *ptr1, *ptr1f;
    Word32 *pRe0, *pIm0;
    Word32 *pRef, *pImf;
    Word16 i;
    Word32 L_temp0, L_temp1, La, Ls, L_temp;
    Word16 s_sRe, s_aIm;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16 +  5 * SIZE_Word32 + 6 * SIZE_Ptr), "dummy");
#endif
    ptr1  = ptr0 +81;
    ptr1f = ptr0f +81;

    pRe0 = ptrReIm; 
    pIm0 = pRe0 +16;
    pImf = ptrReImf;
    pRef = pImf -16;

    FOR(i=0; i<n; i++) {
        La = L_add(*pRe0, *pRef);
        Ls = L_sub(*pIm0, *pImf);
        s_sRe = round_fx(L_shl(L_sub(*pRe0, *pRef),Qfft16));
        s_aIm = round_fx(L_shl(L_add(*pIm0, *pImf),Qfft16));

        L_temp  = L_mult(*ptr_cos, s_aIm);
        L_temp  = L_msu(L_temp,*ptr_sin, s_sRe);
        L_temp  = L_shr(L_temp,Qfft16);
        L_temp0 = L_add(La, L_temp);
        *ptr0   = round_fx(L_shl(L_temp0, Qfft16_1)); move16();
        L_temp1 = L_sub(La, L_temp);
        *ptr0f  = round_fx(L_shl(L_temp1, Qfft16_1)); move16();

        L_temp  = L_mult(*ptr_sin, s_aIm);
        L_temp  = L_mac(L_temp,*ptr_cos, s_sRe);
        L_temp  = L_negate(L_temp);
        L_temp  = L_shr(L_temp,Qfft16);
        L_temp0 = L_add(L_temp, Ls);
        *ptr1   = round_fx(L_shl(L_temp0, Qfft16_1)); move16();
        L_temp1 = L_sub(L_temp, Ls);
        *ptr1f  = round_fx(L_shl(L_temp1, Qfft16_1)); move16();

        ptr_cos += 5;
        ptr_sin += 5;

        pRe0++; pRef--;
        pIm0++; pImf--;

        ptr0  += 5;
        ptr0f -= 5;
        ptr1  += 5;
        ptr1f -= 5;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* fft_5_format_2
* subroutine called by fixDoRFFTx
* Do 5-points complex FFT
**************************************************************************/
static void fft_5_format_2(Word32 *pRe, Word32 *pIm, Word16 *psRe, Word16 *psIm, Word16 QVal)
{   
    Word16   ar14, sr14, ai14, si14, ar32, sr32, ai32, si32 ;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (8 * SIZE_Word16 +  0 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    ar14 = add(psRe[1], psRe[4]);
    sr14 = sub(psRe[1], psRe[4]);
    ar32 = add(psRe[3], psRe[2]);
    sr32 = sub(psRe[3], psRe[2]);

    ai14 = add(psIm[1], psIm[4]);
    si14 = sub(psIm[1], psIm[4]);
    ai32 = add(psIm[3], psIm[2]);
    si32 = sub(psIm[3], psIm[2]);

    calc_fft5R(psRe[0], pRe, ar14, si14, ar32, si32, QVal);
    calc_fft5I(psIm[0], pIm, ai14, sr14, ai32, sr32, QVal);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_R0: 
* subroutine called by calc_fft5R & calc_fft5I
* Compute real/imaginary part of a point in 5-complex FTT  : ps0 + ar14 + ar32
**************************************************************************/
static Word32 calc_R0(Word16 ps0, Word32 *pL, Word16 ar14, Word16 ar32, Word16 QVal)
{
    Word32 L_temp0, L_temp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp0 = L_mult0(ps0, 16384);
    L_temp  = L_mac0(L_temp0, ar14, 16384);
    L_temp  = L_mac0(L_temp, ar32, 16384);
    pL[0]   = L_shr(L_temp, QVal); move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp0);
}

/*************************************************************************
* calcR4
* subroutine called by calc_RI4
* Compute real part of a point in 5-complex FTT : L_temp+ arx *CFFT5_R1 +ary *CFFT5_R2
**************************************************************************/
static Word32 calcR4 (Word32 L_temp0, Word16 arx, Word16 ary)
{
    Word32 L_tempR;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_tempR = L_mac0(L_temp0, arx, CFFT5_R1);
    L_tempR = L_mac0(L_tempR, ary, CFFT5_R2);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_tempR);
}

/*************************************************************************
* calcI4
* subroutine called by calc_RI4
* Compute imaginary  part of a point in 5-complex FTT : six* cx +siy * cy;
**************************************************************************/
static Word32 calcI4 (const Word16 cx, const Word16 cy, Word16 six, Word16 siy)
{
    Word32 L_tempI;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_tempI = L_mult0(six, cx);
    L_tempI = L_mac0(L_tempI, siy, cy);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_tempI);
}

/*************************************************************************
* calcRI4
* subroutine called by calc_fft5R & calc_fft5I
* Compute 4 real (or imaginary parts) of 4 points in 5-complex FTT : 
* Compute: (L_temp0+ arx *CFFT5_R1 +ary *CFFT5_R2) +/- (six* cx +siy * cy)
**************************************************************************/
static void calcRI4 (Word32 L_temp0, Word16 arx, Word16 ary, const Word16 cx, 
                     const Word16 cy, Word16 six, Word16 siy, Word32 *L_a, 
                     Word32 *L_s, Word16 QVal)
{
    Word32 L_tempR, L_tempI, L_temp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  3 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_tempR = calcR4 (L_temp0, arx, ary);
    L_tempI = calcI4 (cx, six, cy, siy);
    L_temp  = L_sub(L_tempR, L_tempI); 
    *L_s    = L_shr(L_temp, QVal); move32();
    L_temp  = L_add(L_tempR, L_tempI); 
    *L_a    = L_shr(L_temp, QVal); move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_fft5R
* subroutine called by ffft_5_format_2
* Calculate the real parts of 5 points of 5-points complex FFT
**************************************************************************/
static void calc_fft5R(Word16 ps0, Word32 *pL, Word16 ar14, Word16 si14, 
                       Word16 ar32, Word16 si32, Word16 QVal)
{
    Word32 L_temp0;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp0 = calc_R0(ps0, pL, ar14, ar32, QVal);
    calcRI4(L_temp0, ar14, ar32,CFFT5_I1, si14, -CFFT5_I2, si32, &pL[4], 
            &pL[1], QVal);
    calcRI4(L_temp0, ar32, ar14, CFFT5_I3, si14, CFFT5_I1, si32, &pL[3], 
            &pL[2], QVal);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_fft5I
* subroutine called by ffft_5_format_2
* Calculate the imaginary part of 5-points complex FFT
**************************************************************************/
static void calc_fft5I(Word16 ps0, Word32 *pL, Word16 ar14, Word16 si14, 
                       Word16 ar32, Word16 si32, Word16 QVal)
{
    Word32 L_temp0;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 +  1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp0 = calc_R0(ps0, pL, ar14, ar32, QVal);
    calcRI4(L_temp0, ar14, ar32,CFFT5_I1, si14, -CFFT5_I2, si32, &pL[1], &pL[4], QVal);
    calcRI4(L_temp0, ar32, ar14, CFFT5_I3, si14, CFFT5_I1, si32, &pL[2], &pL[3], QVal);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* fft_16x_format_2
* subroutine called by fixDoRFFTx
* Do 16-points complex FFT for 80-points complex FFT
**************************************************************************/
static void fft_16x_format_2(Word32 *ReIm, Word16 QVal)
{
    Word32 *Re0, *Im0;
    Word16 s_sumReIm[2*16], *s_sumRe, *s_sumIm;
    Word16 s_sumReIm4[2*14], *s_sumRe4, *s_sumIm4;
    Word16 shift;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1 + 32 + 28) * SIZE_Word16 +  0 * SIZE_Word32 + 6 * SIZE_Ptr), "dummy");
#endif
    QVal= sub(QVal, 2);
    Re0 = ReIm;
    Im0 = ReIm+16;

    s_sumRe = s_sumReIm;
    s_sumIm = s_sumReIm+16;
    s_calcSum2(Re0, s_sumRe, QVal);
    s_calcSum2(Im0, s_sumIm, QVal);

    s_sumRe4 = s_sumReIm4;
    s_sumIm4 = s_sumReIm4+14;
    s_calcSum4RI(s_sumRe, s_sumRe4);
    s_calcSum4RI(s_sumIm, s_sumIm4);

    shift = sub(QVal,1);
    s_fft16R(s_sumReIm4, s_sumReIm, Re0, shift);
    s_fft16I(s_sumReIm4, s_sumReIm, Im0, shift);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* fft_16x_format_2i
* subroutine called by fixDORiFFTx
* Do 16-points complex FFT for 80-points complex iFFT
**************************************************************************/
static void fft_16x_format_2i(Word32 *ReIm, Word16 QVal, Word16 *x, Word16 x_q)
{
    Word32 *Re0, *Im0;
    Word16 s_sumReIm[2*16], *s_sumRe, *s_sumIm;
    Word16 s_sumReIm4[2*14], *s_sumRe4, *s_sumIm4;
    Word16 shift;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((1 + 32 + 28) * SIZE_Word16 +  0 * SIZE_Word32 + 6 * SIZE_Ptr), "dummy");
#endif
    QVal= sub(QVal, 2);
    Re0 = ReIm;
    Im0 = ReIm+16;

    s_sumRe = s_sumReIm;
    s_sumIm = s_sumReIm+16;
    s_calcSum2(Re0, s_sumRe, QVal);
    s_calcSum2(Im0, s_sumIm, QVal);

    s_sumRe4 = s_sumReIm4;
    s_sumIm4 = s_sumReIm4+14;
    s_calcSum4RI(s_sumRe, s_sumRe4);
    s_calcSum4RI(s_sumIm, s_sumIm4);

    shift = sub(add(QVal,x_q),11);
    s_fft16Ri(s_sumReIm4, s_sumReIm, shift, x);
    s_fft16Ii(s_sumReIm4, s_sumReIm, shift, x+1);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* s_calcSum2
*
* subroutine called by fft_16x_format_2 and fft_16x_format_2i
* compute and store 8 sums and differences of 2 points distants from 8 
* (e.g. Re[0]+/- Re[8],  Re[1]+/- Re[9],  ...
**************************************************************************/
static void s_calcSum2(Word32 *Re, Word16 *sumRe, Word16 QVal)
{
    Word16 *ptrS;
    Word32 *ptr0, *ptr1, L_tmp;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  1 * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    ptr0 = Re;
    ptr1 = Re + 8;
    ptrS = sumRe;
    FOR(i=0; i<8; i++)
    {
        L_tmp   = L_add(*ptr0, *ptr1);
        *ptrS++ = round_fx(L_shl(L_tmp, QVal)); move16();
        L_tmp   = L_sub(*ptr0, *ptr1); 
        *ptrS++ = round_fx(L_shl(L_tmp, QVal)); move16();
        ptr0 += 1;
        ptr1 += 1;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* s_calcSum4RI
*
* subroutine called by fft_16x_format_2 and fft_16x_format_2i
* compute and store 4 sums and differences of 4 points 
* from the sums/differences of 2 points computed by s_calcSum2
**************************************************************************/
static void s_calcSum4RI(Word16 *ReIm, Word16 *sumReIm)
{
    Word16 *ptrS;
    Word16 *ptr0, *ptr1;
    Word16 i;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  0 * SIZE_Word32 + 3 * SIZE_Ptr), "dummy");
#endif
    ptr0 = ReIm;
    ptr1 = ptr0 + 8;
    ptrS = sumReIm;

    /* r0, r8, r4, r12 */
    *ptrS++ = sub(*ptr0, *ptr1); move16();
    *ptrS++ = add(*ptr0, *ptr1); move16();
    ptr0 += 2;
    ptr1 += 6;

    /* r1, r9, r7, r15 */
    FOR(i=0; i<3; i++)
    {
        *ptrS   = sub(*ptr0, *ptr1); move16();
        ptrS[3] = add(*ptr0, *ptr1); move16();
        ptrS++;
        ptr0++;
        ptr1++;
        *ptrS++ = add(*ptr0, *ptr1); move16();
        *ptrS++ = sub(*ptr0, *ptr1); move16();
        ptr0++;
        ptr1 -= 3;
        ptrS++;
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* s_fft16R
*  subroutine called by fft_16x_format_2 
* Calculate the real part of 16-points complex FFT for 80-points complex FFT
**************************************************************************/
static void s_fft16R(Word16 *sumReIm4, Word16 *sumReIm, Word32 *Re, Word16 shift)
{
    Word32 L_temp0, L_temp0b, L_temp1, L_temp2, L_temp3, L_temp2b, L_temp3b, L_temp4;
    Word32 L_tempa, L_tempb, L_tempc;
    Word16 *sumRe, *sumIm;
    Word16 *sumRe4, *sumIm4;

    Word16 shift1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  11 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    shift1 = sub(shift,1);

    sumRe = sumReIm;
    sumIm = sumReIm+16;

    sumRe4 = sumReIm4;
    sumIm4 = sumReIm4+14;

    /* D0 + D2 */
    L_tempa = L_mult0(sumRe4[1], 16384);
    L_temp0 = L_shr(L_mac0(L_tempa, sumRe4[6+3], 16384), shift1);
    /* D1 + D3*/
    L_tempb = L_mult0(sumRe4[2+3], 16384);
    L_temp1 = L_shr(L_mac0(L_tempb, sumRe4[2+4*2+3], 16384), shift1);
    Re[0]   = L_add(L_temp0, L_temp1); move32();
    Re[8]   = L_sub(L_temp0, L_temp1); move32();

    /* D0 - D2 */
    L_temp0 = L_shr(L_msu0(L_tempa, sumRe4[6+3], 16384), shift1); 
    /* A'1 - A'3 */
    L_tempc = L_mult0(sumIm4[2], 16384);
    L_temp2 = L_shr(L_msu0(L_tempc, sumIm4[2+4*2], 16384), shift1); 
    Re[4]   = L_add(L_temp0, L_temp2); move32();
    Re[12]  = L_sub(L_temp0, L_temp2); move32();

    /* D1 - D3 */
    L_temp0 = axplusby0 (sumRe4[2+3], sumRe4[2+2*4+3], C_fx81, -C_fx81);
    /* A'1 + A'3 */
    L_temp1 = axplusby0 (sumIm4[2], sumIm4[2+4*2], C_fx81, C_fx81); 

    /*  c81 * (D1 - D3 + (A'1 + A'3)) */
    L_temp2 = L_shr(L_add(L_temp0, L_temp1), shift);  
    /*  c81 * (D1 - D3 - (A'1 + A'3)) */
    L_temp2b = L_shr(L_sub(L_temp0, L_temp1), shift);  

    /*  A0 +A'2*/
    L_tempa = L_mult(sumRe4[0], 16384);
    L_temp3 = L_shr(L_mac(L_tempa, sumIm4[2+4], 16384), shift);
    /*  A0 -A'2*/
    L_temp3b = L_shr(L_msu(L_tempa, sumIm4[2+4], 16384), shift);
    Re[2]   = L_add(L_temp3, L_temp2); move32();
    Re[14]  = L_add(L_temp3b, L_temp2b); move32();
    Re[6]   = L_sub(L_temp3b, L_temp2b); move32();
    Re[10]  = L_sub(L_temp3, L_temp2); move32();

    /* c81* (C2 -B'2)*/
    L_temp0 = axplusby0(sumRe4[2+4+2],  sumIm4[2+4+1], C_fx81, -C_fx81 );
    /* c162* (C1 - B'3)*/
    /* c165* (C3 - B'1)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2], sumIm4[2+2*4+1], C_fx162, -C_fx162,
                           sumRe4[2+2*4+2], sumIm4[2+1], -C_fx165, C_fx165);
    L_tempa = L_mult(sumRe[1], 16384);
    L_temp1 = L_msu(L_tempa, sumIm[9], 16384);
    L_temp4 = L_sub(L_temp1 , L_temp0);

    Re[3]   = L_shr(L_add(L_temp4, L_temp3), shift); move32();
    Re[11]  = L_shr(L_sub(L_temp4, L_temp3), shift); move32();

    /* c81* (C2 +B'2)*/
    L_temp0b = axplusby0(sumRe4[2+4+2], sumIm4[2+4+1], C_fx81, C_fx81);
    /* c162* (C1 + B'3)*/
    /* c165* (C3 + B'1)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2], sumIm4[2+2*4+1] , C_fx162, C_fx162,
                           sumRe4[2+2*4+2], sumIm4[2+1], -C_fx165, -C_fx165);
    L_temp2 = L_mac(L_tempa, sumIm[9], 16384);
    L_temp4 = L_sub(L_temp2, L_temp0b);

    Re[13]  = L_shr(L_add(L_temp4, L_temp3), shift); move32();
    Re[5]   = L_shr(L_sub(L_temp4, L_temp3), shift); move32();

    /* c162* (C3 + B'1)*/
    /* c165* (C1 + B'3)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2*4+2], sumIm4[2+1], C_fx162, C_fx162,
                           sumRe4[2+2], sumIm4[2+2*4+1], C_fx165, C_fx165);
    L_temp4 = L_add(L_temp2, L_temp0b);

    Re[1]   = L_shr(L_add(L_temp4, L_temp3), shift); move32();
    Re[9]   = L_shr(L_sub(L_temp4, L_temp3), shift); move32();

    /* c162* (C3-B'1)*/
    /* c165* (C1 - B'3)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2*4+2], sumIm4[2+1], C_fx162, -C_fx162,
                           sumRe4[2+2], sumIm4[2+2*4+1], C_fx165, -C_fx165);
    L_temp4 = L_add(L_temp1, L_temp0);
    L_temp3 = L_shr(L_temp3, shift);
    L_temp4 = L_shr(L_temp4, shift);
    Re[7]   = L_sub(L_temp4, L_temp3); move32();
    Re[15]  = L_add(L_temp4, L_temp3); move32();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* s_fft16I
*  subroutine called by fft_16x_format_2 
* Calculate the imaginary part of 16-points complex FFT for 80-points complex FFT
**************************************************************************/
static void s_fft16I(Word16 *sumReIm4, Word16 *sumReIm, Word32 *Im, Word16 shift)
{
    Word32 L_tempa, L_temp0, L_temp1, L_temp2, L_temp3, L_temp4;
    Word32 L_tempb, L_temp0b, L_temp2b, L_temp3b;
    Word16 *sumRe, *sumIm;
    Word16 *sumRe4, *sumIm4;
    Word16 shift1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  10 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    shift1 = sub(shift,1);

    sumRe  = sumReIm;
    sumIm  = sumReIm+16;
    sumRe4 = sumReIm4;
    sumIm4 = sumReIm4+14;

    /* D'0 + D'2 */
    L_tempa = L_mult0(sumIm4[1], 16384);

    L_temp0 = L_shr(L_mac0(L_tempa, sumIm4[6+3], 16384), shift1); 
    /* D1 + D3*/
    L_temp1 = L_mult0(sumIm4[2+3], 16384);
    L_temp1 = L_shr(L_mac0(L_temp1, sumIm4[2+4*2+3], 16384), shift1); 
    Im[0]   = L_add(L_temp0, L_temp1); move32();
    Im[8]   = L_sub(L_temp0, L_temp1); move32();

    /* D'0 - D'2 */
    L_temp0 = L_shr(L_msu0(L_tempa, sumIm4[6+3], 16384), shift1); 
    /* A1 - A3 */
    L_temp2 = L_mult0(sumRe4[2], 16384); 
    L_temp2 = L_shr(L_msu0(L_temp2, sumRe4[2+4*2], 16384), shift1);

    Im[4]   = L_sub(L_temp0, L_temp2); move32();
    Im[12]  = L_add(L_temp0, L_temp2); move32();

    /* D'1 - D'3 */
    L_temp0 = axplusby0(sumIm4[2+3], sumIm4[2+2*4+3], C_fx81, -C_fx81);
    /* A1 + A3 */
    L_temp1 = axplusby0(sumRe4[2], sumRe4[2+4*2], C_fx81, C_fx81); 
    /*  c81 * (D'1 - D'3 + (A1 + A3)) */
    L_temp2 = L_shr(L_add(L_temp0, L_temp1), shift);  
    /*  c81 * (D'1 - D'3 - (A1 + A3)) */
    L_temp2b = L_shr(L_sub(L_temp0, L_temp1), shift);  

    /*  A'0 +A2*/
    L_tempa = L_mult(sumIm4[0], 16384);
    L_temp3 = L_shr(L_mac(L_tempa , sumRe4[2+4], 16384), shift);
    /*  A'0 -A2*/
    L_temp3b = L_shr(L_msu(L_tempa , sumRe4[2+4], 16384), shift);

    Im[14]  = L_add(L_temp3, L_temp2); move32();
    Im[2]   = L_add(L_temp3b, L_temp2b); move32();
    Im[10]  = L_sub(L_temp3b, L_temp2b); move32();
    Im[6]   = L_sub(L_temp3, L_temp2); move32();

    /* c81* (C2 -B'2)*/
    L_temp0 = axplusby0(sumIm4[2+4+2], sumRe4[2+4+1], C_fx81, -C_fx81);
    /* c162* (C1 - B'3)*/
    /* c165* (C3 - B'1)*/
    L_temp3 = L_shr(axpmy_bzpmt0(sumIm4[2+2], sumRe4[2+2*4+1], C_fx162, -C_fx162,
                                 sumIm4[2+2*4+2], sumRe4[2+1], -C_fx165, C_fx165), shift);
    L_tempa = L_mult(sumIm[1],16384);
    L_temp1 = L_msu(L_tempa, sumRe[9], 16384);
    L_temp4 = L_shr(L_sub(L_temp1, L_temp0), shift);

    Im[13]  = L_add(L_temp4, L_temp3); move32();
    Im[5]   = L_sub(L_temp4, L_temp3); move32();

    /* c81* (C2 +B'2)*/
    L_temp0b = axplusby0(sumIm4[2+4+2], sumRe4[2+4+1], C_fx81, C_fx81);
    /* c162* (C1 + B'3)*/
    /* c165* (C3 + B'1)*/
    L_temp3 = L_shr(axpmy_bzpmt0(sumIm4[2+2], sumRe4[2+2*4+1], C_fx162, C_fx162,
                                 sumIm4[2+2*4+2], sumRe4[2+1], -C_fx165, -C_fx165), shift);
    L_tempb = L_mac(L_tempa, sumRe[9], 16384);
    L_temp4 = L_shr(L_sub(L_tempb, L_temp0b), shift);

    Im[3]   = L_add(L_temp4, L_temp3); move32();
    Im[11]  = L_sub(L_temp4, L_temp3); move32();

    /* c162* (C3 + B'1)*/
    /* c165* (C1 + B'3)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2*4+2], sumRe4[2+1], C_fx162, C_fx162,
                           sumIm4[2+2], sumRe4[2+2*4+1], C_fx165 ,C_fx165);
    L_temp4 = L_add(L_tempb, L_temp0b);
    L_temp3 = L_shr(L_temp3, shift);
    L_temp4 = L_shr(L_temp4, shift);

    Im[15]  = L_add(L_temp4, L_temp3); move32();
    Im[7]   = L_sub(L_temp4, L_temp3); move32();

    /* c162* (C3-B'1)*/
    /* c165* (C1 - B'3)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2*4+2], sumRe4[2+1], C_fx162,-C_fx162,
                           sumIm4[2+2], sumRe4[2+2*4+1], C_fx165, -C_fx165);
    L_temp4 = L_add(L_temp1, L_temp0);
    L_temp3 = L_shr(L_temp3, shift);
    L_temp4 = L_shr(L_temp4, shift);

    Im[9]   = L_sub(L_temp4, L_temp3); move32();
    Im[1]   = L_add(L_temp4, L_temp3); move32();

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;

}

/*************************************************************************
* s_fft16Ri
*  subroutine called by fft_16x_format_2i 
* Calculate the real part of 16-points complex FFT for 80-points complex iFFT
**************************************************************************/
static void s_fft16Ri(Word16 *sumReIm4, Word16 *sumReIm, Word16 shift, Word16 *x)
{
    Word32 L_temp0, L_temp0b, L_temp1, L_temp2, L_temp3, L_temp2b, L_temp3b, L_temp4;
    Word32 L_tempa, L_tempb, L_tempc;
    Word16 *sumRe, *sumIm;
    Word16 *sumRe4, *sumIm4;

    Word16 shift1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  11 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    shift1 = add(shift,1);

    sumRe = sumReIm;
    sumIm = sumReIm+16;

    sumRe4 = sumReIm4;
    sumIm4 = sumReIm4+14;

    /* D0 + D2 */
    L_tempa = L_mult0(sumRe4[1], 16384);
    L_temp0 = L_mac0(L_tempa, sumRe4[6+3], 16384);
    /* D1 + D3*/
    L_tempb = L_mult0(sumRe4[2+3], 16384);
    L_temp1 = L_mac0(L_tempb, sumRe4[2+4*2+3], 16384);

    L_temp0 = L_shr(L_temp0, shift);
    L_temp1 = L_shr(L_temp1, shift);
    x[0]      = shl(mult(G_FX,round_fx(L_add(L_temp0, L_temp1))), 1); move16();
    x[8*2*5]  = shl(mult(G_FX,round_fx(L_sub(L_temp0, L_temp1))), 1); move16();

    /* D0 - D2 */
    L_temp0 = L_msu0(L_tempa, sumRe4[6+3], 16384); 
    /* A'1 - A'3 */
    L_tempc = L_mult0(sumIm4[2], 16384);
    L_temp2 = L_msu0(L_tempc, sumIm4[2+4*2], 16384); 

    L_temp0 = L_shr(L_temp0, shift);
    L_temp2 = L_shr(L_temp2, shift);
    x[4*2*5]  = shl(mult(G_FX,round_fx(L_add(L_temp0, L_temp2))), 1); move16();
    x[12*2*5] = shl(mult(G_FX,round_fx(L_sub(L_temp0, L_temp2))), 1); move16();

    /* D1 - D3 */
    L_temp0 = axplusby0(sumRe4[2+3], sumRe4[2+2*4+3], C_fx81, -C_fx81);
    /* A'1 + A'3 */
    L_temp1 = axplusby0(sumIm4[2], sumIm4[2+4*2], C_fx81, C_fx81); 

    /*  c81 * (D1 - D3 + (A'1 + A'3)) */
    L_temp2 = L_add(L_temp0, L_temp1);  
    /*  c81 * (D1 - D3 - (A'1 + A'3)) */
    L_temp2b = L_sub(L_temp0, L_temp1);  

    /*  A0 +A'2*/
    L_tempa = L_mult(sumRe4[0], 16384);
    L_temp3 = L_mac(L_tempa, sumIm4[2+4], 16384);
    /*  A0 -A'2*/
    L_temp3b = L_msu(L_tempa, sumIm4[2+4], 16384);

    L_temp3  = L_shr(L_temp3, shift1);
    L_temp2  = L_shr(L_temp2, shift1);
    L_temp3b = L_shr(L_temp3b, shift1);
    L_temp2b = L_shr(L_temp2b, shift1);
    x[2*2*5]  = shl(mult(G_FX,round_fx(L_add(L_temp3, L_temp2))), 1); move16();
    x[14*2*5] = shl(mult(G_FX,round_fx(L_add(L_temp3b, L_temp2b))), 1); move16();
    x[6*2*5]  = shl(mult(G_FX,round_fx(L_sub(L_temp3b, L_temp2b))), 1); move16();
    x[10*2*5] = shl(mult(G_FX,round_fx(L_sub(L_temp3, L_temp2))), 1); move16();

    /* c81* (C2 -B'2)*/
    L_temp0 = axplusby0(sumRe4[2+4+2],  sumIm4[2+4+1], C_fx81, -C_fx81 );
    /* c162* (C1 - B'3)*/
    /* c165* (C3 - B'1)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2], sumIm4[2+2*4+1], C_fx162, -C_fx162,
                           sumRe4[2+2*4+2], sumIm4[2+1], -C_fx165, C_fx165);
    L_tempa = L_mult(sumRe[1], 16384);
    L_temp1 = L_msu(L_tempa, sumIm[9], 16384);
    L_temp4 = L_sub(L_temp1 , L_temp0);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[3*2*5]  = shl(mult(G_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[11*2*5] = shl(mult(G_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c81* (C2 +B'2)*/
    L_temp0b = axplusby0(sumRe4[2+4+2], sumIm4[2+4+1], C_fx81, C_fx81);
    /* c162* (C1 + B'3)*/
    /* c165* (C3 + B'1)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2], sumIm4[2+2*4+1] , C_fx162, C_fx162,
                           sumRe4[2+2*4+2], sumIm4[2+1], -C_fx165, -C_fx165);
    L_temp2 = L_mac(L_tempa, sumIm[9], 16384);
    L_temp4 = L_sub(L_temp2, L_temp0b);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[13*2*5] = shl(mult(G_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[5*2*5]  = shl(mult(G_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c162* (C3 + B'1)*/
    /* c165* (C1 + B'3)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2*4+2], sumIm4[2+1], C_fx162, C_fx162,
                           sumRe4[2+2], sumIm4[2+2*4+1], C_fx165, C_fx165);
    L_temp4 = L_add(L_temp2, L_temp0b);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[1*2*5]  = shl(mult(G_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[9*2*5]  = shl(mult(G_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c162* (C3-B'1)*/
    /* c165* (C1 - B'3)*/
    L_temp3 = axpmy_bzpmt0(sumRe4[2+2*4+2], sumIm4[2+1], C_fx162, -C_fx162,
                           sumRe4[2+2], sumIm4[2+2*4+1], C_fx165, -C_fx165);
    L_temp4 = L_add(L_temp1, L_temp0);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[7*2*5]  = shl(mult(G_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();
    x[15*2*5] = shl(mult(G_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* s_fft16Ii
*  subroutine called by fft_16x_format_2i 
* Calculate the imaginary part of 16-points complex FFT for 80-points complex iFFT
**************************************************************************/
static void s_fft16Ii(Word16 *sumReIm4, Word16 *sumReIm, Word16 shift, Word16 *x)
{
    Word32 L_tempa, L_temp0, L_temp1, L_temp2, L_temp3, L_temp4;
    Word32 L_tempb, L_temp0b, L_temp2b, L_temp3b;
    Word16 *sumRe, *sumIm;
    Word16 *sumRe4, *sumIm4;
    Word16 shift1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word16 +  10 * SIZE_Word32 + 4 * SIZE_Ptr), "dummy");
#endif
    shift1 = add(shift,1);

    sumRe = sumReIm;
    sumIm = sumReIm+16;

    sumRe4 = sumReIm4;
    sumIm4 = sumReIm4+14;

    /* D'0 + D'2 */
    L_tempa = L_mult0(sumIm4[1], 16384);
    L_temp0 = L_mac0(L_tempa, sumIm4[6+3], 16384); 
    /* D1 + D3*/
    L_temp1 = L_mult0(sumIm4[2+3], 16384);
    L_temp1 = L_mac0(L_temp1, sumIm4[2+4*2+3], 16384); 

    L_temp0 = L_shr(L_temp0, shift);
    L_temp1 = L_shr(L_temp1, shift);
    x[0*2*5]  = shl(mult(NG_FX,round_fx(L_add(L_temp0, L_temp1))), 1); move16();
    x[8*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp0, L_temp1))), 1); move16();

    /* D'0 - D'2 */
    L_temp0 = L_msu0(L_tempa, sumIm4[6+3], 16384); 
    /* A1 - A3 */
    L_temp2 = L_mult0(sumRe4[2], 16384); 
    L_temp2 = L_msu0(L_temp2, sumRe4[2+4*2], 16384); 

    L_temp0 = L_shr(L_temp0, shift);
    L_temp2 = L_shr(L_temp2, shift);
    x[4*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp0, L_temp2))), 1); move16();
    x[12*2*5] = shl(mult(NG_FX,round_fx(L_add(L_temp0, L_temp2))), 1); move16();

    /* D'1 - D'3 */
    L_temp0 = axplusby0(sumIm4[2+3], sumIm4[2+2*4+3], C_fx81, -C_fx81);
    /* A1 + A3 */
    L_temp1 = axplusby0(sumRe4[2], sumRe4[2+4*2], C_fx81, C_fx81); 
    /*  c81 * (D'1 - D'3 + (A1 + A3)) */
    L_temp2 = L_add(L_temp0, L_temp1);  
    /*  c81 * (D'1 - D'3 - (A1 + A3)) */
    L_temp2b = L_sub(L_temp0, L_temp1);  

    /*  A'0 +A2*/
    L_tempa = L_mult(sumIm4[0], 16384);
    L_temp3 = L_mac(L_tempa , sumRe4[2+4], 16384);
    /*  A'0 -A2*/
    L_temp3b = L_msu(L_tempa , sumRe4[2+4], 16384);

    L_temp2  = L_shr(L_temp2, shift1);
    L_temp3  = L_shr(L_temp3, shift1);
    L_temp2b = L_shr(L_temp2b, shift1);
    L_temp3b = L_shr(L_temp3b, shift1);
    x[14*2*5] = shl(mult(NG_FX,round_fx(L_add(L_temp3, L_temp2))), 1); move16();
    x[2*2*5]  = shl(mult(NG_FX,round_fx(L_add(L_temp3b, L_temp2b))), 1); move16();
    x[10*2*5] = shl(mult(NG_FX,round_fx(L_sub(L_temp3b, L_temp2b))), 1); move16();
    x[6*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp3, L_temp2))), 1); move16();

    /* c81* (C2 -B'2)*/
    L_temp0 = axplusby0(sumIm4[2+4+2], sumRe4[2+4+1], C_fx81, -C_fx81);
    /* c162* (C1 - B'3)*/
    /* c165* (C3 - B'1)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2], sumRe4[2+2*4+1], C_fx162, -C_fx162,
                           sumIm4[2+2*4+2], sumRe4[2+1], -C_fx165, C_fx165);
    L_tempa = L_mult(sumIm[1],16384);
    L_temp1 = L_msu(L_tempa, sumRe[9], 16384);
    L_temp4 = L_sub(L_temp1, L_temp0);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[13*2*5] = shl(mult(NG_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[5*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c81* (C2 +B'2)*/
    L_temp0b = axplusby0(sumIm4[2+4+2], sumRe4[2+4+1], C_fx81, C_fx81);
    /* c162* (C1 + B'3)*/
    /* c165* (C3 + B'1)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2], sumRe4[2+2*4+1], C_fx162, C_fx162,
                           sumIm4[2+2*4+2], sumRe4[2+1], -C_fx165, -C_fx165);
    L_tempb = L_mac(L_tempa, sumRe[9], 16384);
    L_temp4 = L_sub(L_tempb, L_temp0b);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[3*2*5]  = shl(mult(NG_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[11*2*5] = shl(mult(NG_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c162* (C3 + B'1)*/
    /* c165* (C1 + B'3)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2*4+2], sumRe4[2+1], C_fx162, C_fx162,
                           sumIm4[2+2], sumRe4[2+2*4+1], C_fx165 ,C_fx165);
    L_temp4 = L_add(L_tempb, L_temp0b);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[15*2*5] = shl(mult(NG_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();
    x[7*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();

    /* c162* (C3-B'1)*/
    /* c165* (C1 - B'3)*/
    L_temp3 = axpmy_bzpmt0(sumIm4[2+2*4+2], sumRe4[2+1], C_fx162,-C_fx162,
                           sumIm4[2+2], sumRe4[2+2*4+1], C_fx165, -C_fx165);
    L_temp4 = L_add(L_temp1, L_temp0);

    L_temp3 = L_shr(L_temp3, shift1);
    L_temp4 = L_shr(L_temp4, shift1);
    x[9*2*5]  = shl(mult(NG_FX,round_fx(L_sub(L_temp4, L_temp3))), 1); move16();
    x[1*2*5]  = shl(mult(NG_FX,round_fx(L_add(L_temp4, L_temp3))), 1); move16();

#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;

}

/*************************************************************************
* axplusby0
* subroutine called by s_fft16R, s_fft16I, s_fft16Ri, s_fft16Ii
* calculate ca*x+ cb*y
**************************************************************************/
static Word32 axplusby0 (Word16 x, Word16 y, const Word16 ca, const Word16 cb)
{
    Word32 L_temp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word32), "dummy");
#endif
    L_temp = L_mult0(x, ca);
    L_temp = L_mac0(L_temp,  y, cb);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp);
}

/*************************************************************************
* axpmy_bzpmt0
* subroutine called by s_fft16R, s_fft16I, s_fft16Ri, s_fft16Ii
* calculate ca*x+ cay*y+cbz*z + cbt*t
**************************************************************************/
static Word32 axpmy_bzpmt0 (Word16 x, Word16 y, const Word16 cax, const Word16 cay,
                            Word16 z, Word16 t, const Word16 cbz, const Word16 cbt)
{
    Word32 L_temp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (1 * SIZE_Word32), "dummy");
#endif
    L_temp = L_mult0(x, cax);
    L_temp = L_mac0(L_temp,  y, cay);
    L_temp = L_mac0(L_temp,  z, cbz);
    L_temp = L_mac0(L_temp,  t, cbt);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return(L_temp);
}

/*************************************************************************
* calc_RiFFTx
* subroutine called by fixDoRiFFTx
* Calculate real and imaginary part of 80-point complex iFFT
**************************************************************************/
static void calc_RiFFTx(Word16 *x, Word16 *sRe, Word16 *sIm, Word16 norm_shift)
{
    Word16 dataNo, blockNo, QVal;
    Word16 *psRe, *psIm; 
    Word16 tmp, tmp1;

    Word16 *psRe0, *psIm0, *psRe0f, *psIm0f, *ptr0, *ptr0f;
    Word16 *psRef, *psImf; 
    Word16 *ptr_sinw_s1, *ptr_sinw_p1, *ptr_cosw;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (5 * SIZE_Word16 + 0 * SIZE_Word32 + 13 * SIZE_Ptr), "dummy");
#endif
    psRe0  = sRe;
    psIm0  = sIm;
    psRe0f = psRe0 + 79;
    psIm0f = psIm0 + 79;

    ptr0   = x;
    ptr0f  = x + 80;

    QVal   = sub(norm_shift,1);

    tmp    = shl(*ptr0++,QVal);
    tmp1   = shl(*ptr0f--,QVal);
    *psRe0 = add(tmp, tmp1); move16();
    *psIm0 = sub(tmp1, tmp); move16();

    ptr_sinw_s1 = sinw_s1 + 1;
    ptr_sinw_p1 = sinw_p1 + 1;
    ptr_cosw    = cosw + 1;

    FOR (dataNo=1; dataNo<3; dataNo++)
    {   
        psRe  = psRe0++; 
        psIm  = psIm0++;
        psRef = psRe0f--;
        psImf = psIm0f--;

        FOR(blockNo=1;blockNo<16;blockNo++)
        {   
            psRe += 5;
            psIm += 5;
            calc_RiFFT_1_4(ptr0, ptr0f, norm_shift, *ptr_sinw_s1, *ptr_sinw_p1, 
                           *ptr_cosw, psRe, psRef, psIm, psImf);
            ptr0++; ptr0f--;
            psRef -= 5;
            psImf -= 5;
            ptr_sinw_s1++;
            ptr_sinw_p1++; 
            ptr_cosw++; 
        }
        calc_RiFFT_1_4(ptr0, ptr0f, norm_shift, *ptr_sinw_s1, *ptr_sinw_p1, 
                       *ptr_cosw, psRe0, psRef, psIm0, psImf);
        ptr0++;
        ptr0f--;
        ptr_sinw_s1++;
        ptr_sinw_p1++; 
        ptr_cosw++;
    }

    psRe  = psRe0++; 
    psIm  = psIm0++;
    psRef = psRe0f--;
    psImf = psIm0f--;
    FOR(blockNo=1;blockNo<8;blockNo++)
    {   
        psRe += 5;
        psIm += 5;
        calc_RiFFT_1_4(ptr0, ptr0f, norm_shift, *ptr_sinw_s1, *ptr_sinw_p1, 
                       *ptr_cosw, psRe, psRef, psIm, psImf);
        ptr0++;
        ptr0f--;
        psRef -= 5;
        psImf -= 5;
        ptr_sinw_s1++;
        ptr_sinw_p1++; 
        ptr_cosw++; 
    }

    calc_RiFFT_1_2(ptr0, ptr0f, norm_shift, *ptr_sinw_s1, *ptr_sinw_p1, 
                   *ptr_cosw, psRef, psImf);
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_RiFFT_1_4
* subroutine called by calc_RiFFTx
* Calculate real and imaginary part of of 2 points in 80-point complex iFFT
**************************************************************************/
static void calc_RiFFT_1_4(Word16 *x0, Word16 *x0f, Word16 norm_shift, 
                           Word16 sinw_s1, Word16 sinw_p1, Word16 cosw, 
                           Word16 *Re, Word16 *Ref, Word16 *Im, Word16 *Imf)
{
    Word32 L_temp, L_temp1;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 + 2 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp  = L_mult0(cosw, x0[81]);
    L_temp  = L_mac0(L_temp, cosw, x0f[81]);
    L_temp1 = L_mac0(L_temp, sinw_s1,x0f[0]);
    L_temp1 = L_mac(L_temp1, sinw_p1, x0[0]);
    *Ref    = round_fx(L_shl(L_temp1,norm_shift)); move16();

    L_temp1 = L_mult0(sinw_s1,x0[0]);
    L_temp1 = L_mac(L_temp1, sinw_p1, x0f[0]);
    L_temp1 = L_sub(L_temp1, L_temp);
    *Re     = round_fx(L_shl(L_temp1,norm_shift)); move16();

    L_temp  = L_mult0(cosw, x0f[0]);
    L_temp  = L_msu0(L_temp, cosw, x0[0]);
    L_temp1 = L_msu0(L_temp, sinw_s1,x0[81]);
    L_temp1 = L_mac(L_temp1, sinw_p1, x0f[81]);
    *Im     = round_fx(L_shl(L_temp1,norm_shift)); move16();

    L_temp1 = L_msu0(L_temp, sinw_s1,x0f[81]);
    L_temp1 = L_mac(L_temp1, sinw_p1, x0[81]);
    *Imf    = round_fx(L_shl(L_temp1,norm_shift)); move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* calc_RiFFT_1_2
* subroutine called by calc_RiFFTx
* Calculate real and imaginary part of of 1 point in 80-point complex iFFT
**************************************************************************/
static void calc_RiFFT_1_2(Word16 *x0, Word16 *x0f, Word16 norm_shift, 
                           Word16 sinw_s1, Word16 sinw_p1, Word16 cosw, 
                           Word16 *Ref, Word16 *Imf)
{
    Word32 L_temp;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) (0 * SIZE_Word16 + 1 * SIZE_Word32 + 0 * SIZE_Ptr), "dummy");
#endif
    L_temp = L_mult0(cosw, x0[81]);
    L_temp = L_mac0(L_temp , cosw, x0f[81]);
    L_temp = L_mac0(L_temp , sinw_s1, x0f[0]);
    L_temp = L_mac(L_temp , sinw_p1, x0[0]);
    *Ref   = round_fx(L_shl(L_temp,norm_shift)); move16();

    L_temp = L_mult0(cosw, x0f[0]);
    L_temp = L_msu0(L_temp , cosw, x0[0]);
    L_temp = L_msu0(L_temp , sinw_s1, x0f[81]);
    L_temp = L_mac(L_temp , sinw_p1, x0[81]);
    *Imf   = round_fx(L_shl(L_temp,norm_shift)); move16();
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}

/*************************************************************************
* fixDoRiFFTx
*
* Do 160-points real iFFT
**************************************************************************/
void fixDoRiFFTx(Word16 x[], Word16 *x_q)
{   

    Word16 i;

    Word16 dataNo,groupNo;
    Word32 ReIm[160], ReIm2[32], *Re, *Im;
    Word32 *pRe,*pIm, *pLRe,*pLIm;
    Word16 sRe[80],sIm[80];
    Word16 *psRe,*psIm;
    Word32 *pRe0,*pIm0;
    Word16 Qfft5, Qfft51;
    Word16 *ptr_x;
    Word16 norm_shift;
    Word16 *ptr_twiddleRe, *ptr_twiddleIm;
#ifdef DYN_RAM_CNT
    DYN_RAM_PUSH((UWord32) ((6 + 2 * 80) * SIZE_Word16 + (160 + 32) * SIZE_Word32 + 13 * SIZE_Ptr), "dummy");
#endif
    norm_shift = Exp16Array(162,x);
    norm_shift = sub(norm_shift,2);
    *x_q = add(*x_q, norm_shift);

    calc_RiFFTx(x, sRe, sIm, norm_shift);

    pRe0 = ReIm ;
    pIm0 = ReIm +80 ;
    psRe = sRe;
    psIm = sIm;
    FOR (groupNo=0; groupNo<16; groupNo++)
    {  
        fft_5_format_2(pRe0, pIm0, psRe, psIm, 14);   
        pRe0 += 5;
        pIm0 += 5;
        psRe += 5;
        psIm += 5;
    }

    Qfft5  = Exp32Array(160,ReIm);
    Qfft5  = sub(Qfft5, 1);
    Qfft51 = sub(Qfft5, 1);

    Re   = ReIm;
    Im   = ReIm + 80;
    pRe0 = ReIm2;
    pIm0 = ReIm2+16;
    pRe  = Re++;
    pIm  = Im++ ;
    FOR(i=0; i<16; i++)
    {
        *pRe0++ = *pRe; move32();
        *pIm0++ = *pIm; move32();
        pRe += 5;
        pIm += 5;
    }
    ptr_x = x;
    fft_16x_format_2i(ReIm2, Qfft5, ptr_x, *x_q);
    ptr_twiddleRe = twiddleRe;
    ptr_twiddleIm = twiddleIm;

    FOR (dataNo=1; dataNo<5; dataNo++)
    {   
        ptr_x += 2;
        pRe  = Re++;
        pIm  = Im++;
        pLRe = ReIm2;
        pLIm = ReIm2+16;
        *pLRe++ = *pRe; move32();
        *pLIm++ = *pIm; move32();
        twiddleReIm(pLRe, pLIm, pRe, pIm, ptr_twiddleRe+1, ptr_twiddleIm+1,
                    Qfft5, Qfft51);
        ptr_twiddleRe += 16;
        ptr_twiddleIm += 16;
        fft_16x_format_2i(ReIm2, Qfft5, ptr_x, *x_q);
    }
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    return;
}
#endif /*LAYER_STEREO*/
