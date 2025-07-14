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

#include "pcmswb_common.h"
#include "bwe_mdct_table.h"
#include "bwe_mdct.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

static void cfft(Word16 *x1, Word16 *x2, Word16 sign);

void bwe_mdct (
               Word16 * mem,        /* (i): old input samples    */
               Word16 * input,      /* (i): input samples        */
               Word16 * ykr,        /* (o): MDCT coefficients    */
               Word16 * norm_shift  /* (o): normalization factor */
               )
{
    Word32   ACC0;               /* ACC */

    Word16   xin[MDCT2_L_WIN];
    Word16   ycr[MDCT2_L_WIN4];
    Word16   yci[MDCT2_L_WIN4];

    const Word16 *ptr_h1;        /* pointer on window */
    const Word16 *ptr_h2;        /* pointer on window */
    Word16   *ptr_x1;            /* pointer on input samples */
    Word16   *ptr_x2;            /* pointer on input samples */
    Word16   *ptr_ycr;           /* pointer on ycr */
    Word16   *ptr_yci;           /* pointer on yci */

    Word16   k;
    Word16   i;
    Word16   tmp16_ycr;
    Word16   tmp16_norm_shift;

    const Word16   *ptr_wsin, *ptr_wcos;
    Word16   *ptr_x3, *ptr_x4;

    /*****************************/
#ifdef DYN_RAM_CNT
    {
        UWord32 ssize;
        ssize = (UWord32) (10 * SIZE_Ptr);
        ssize += (UWord32) ((MDCT2_L_WIN + 2 * MDCT2_L_WIN4 + 4) * SIZE_Word16);
        ssize += (UWord32) (1 * SIZE_Word32);
        DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    /*****************************/  

    /********************************************************************************/
    /* MDCT Computation                                                             */
    /********************************************************************************/

    /* form block of length N */
    mov16(MDCT2_L_WIN2, mem, xin);
    mov16(MDCT2_L_WIN2, input, &xin[MDCT2_L_WIN2] );

    /* Step 1 --> Pre-scaling of input signal
    compute norm_shift               */
    *norm_shift = Exp16Array (MDCT2_L_WIN, xin);
    array_oper(MDCT2_L_WIN, *norm_shift, xin, xin, &shl);

    /* Step 2 --> Calculate zn =  (y2n-yN/2-1-2n) + j(yN-1-2n+yN+2+2n) (complex terms), 
    for n=0...N/4-1                                         */

    ptr_h1 = MDCT_h_swb;                          /* Start of the window */
    ptr_h2 = MDCT_h_swb + MDCT2_L_WIN2 - 1; /* End of the window   */   
    ptr_x1 = xin;

    ptr_x2 = xin + MDCT2_L_WIN2 - 1;   
    ptr_x3 = xin + MDCT2_L_WIN2;   
    ptr_x4 = xin + MDCT2_L_WIN - 1;   

    ptr_yci = yci;
    ptr_ycr = ycr;


    FOR (i = 0; i < MDCT2_L_WIN4; i++)
    {
        ACC0 = L_mult(*ptr_h1, *ptr_x1);

        *ptr_ycr++ = msu_r(ACC0, *ptr_h2, *ptr_x2);

        move16();


        ACC0 = L_mult(*ptr_h1, *ptr_x4);

        *ptr_yci++ = mac_r(ACC0, *ptr_h2, *ptr_x3);

        move16();

        ptr_h1 += 2;   
        ptr_h2 -= 2;   
        ptr_x1 += 2;   
        ptr_x2 -= 2;   
        ptr_x3 += 2;   
        ptr_x4 -= 2;   
    }

    /* Step 3 --> Calculate z'n = zn.WN^n, for n=0...N/4-1 */

    ptr_yci = yci;
    ptr_ycr = ycr;

    ptr_wsin = MDCT_wsin_swb;                      
    ptr_wcos = MDCT_wsin_swb + MDCT2_L_WIN4; 


    FOR (k = 0; k < MDCT2_L_WIN4; k++)
    {
        tmp16_ycr = *ptr_ycr;

        ACC0 = L_mult0(tmp16_ycr, *ptr_wcos);
        ACC0 = L_msu0(ACC0, yci[k], *ptr_wsin);
        *ptr_ycr++ = round_fx(ACC0);
        move16();

        ACC0 = L_mult0(tmp16_ycr, *ptr_wsin);
        ACC0 = L_mac0(ACC0, yci[k], *ptr_wcos);
        *ptr_yci++ = round_fx(ACC0);
        move16();

        ptr_wsin++;
        ptr_wcos--;
    }

    /* Step 3 --> Inverse FFT of size N/4: Z'k = FFT-1 z'n, for k=0...N/4-1 */
    cfft(ycr, yci, -1);
    /* Step 4 --> Calculate Zk = 1/80 . ((-1)^k+1.W8^-1.W4N^(4k+1)) . Z'k

    Step 5 --> Rearranging results:
    Y2k       = Im[Zk]
    Y2(k+N/4) = Re[Zk]

    Since Y2(k+N/4) =-Y(N/2-1-2k), results are actually presented as follows:
    Y2k       = Im[Zk]
    YN/2-1-2k = -Re[Zk]                                             

    Steps 4 & 5 are integrated below in a single step */

    ptr_x1 = ykr;
    ptr_x2 = ykr + MDCT2_L_WIN2 - 1;   

    FOR (k = 0; k < MDCT2_L_WIN4; k++)
    {
        tmp16_ycr = ycr[k];


        /* symetry of coeff k-1 and N-k */

        ACC0 = L_mult0(yci[k], MDCT_weti_swb[k]);         /* weti in Q21 */
        ACC0 = L_msu0(ACC0, tmp16_ycr, MDCT_wetr_swb[k]); /* wetr in Q21 */
        *ptr_x2-- = round_fx(ACC0); move16();
        ptr_x2--;

        ACC0 = L_mult0(tmp16_ycr, MDCT_weti_swb[k]);       /* weti in Q21 */
        ACC0 = L_mac0(ACC0, yci[k], MDCT_wetr_swb[k]);     /* wetr in Q21 */
        *ptr_x1++ = round_fx(ACC0); move16();
        ptr_x1++;
    }

    /* Step 6 --> Post-scaling of MDCT coefficient
    compute norm_shift               */
    tmp16_norm_shift = Exp16Array (MDCT2_L_WIN2, ykr);
    array_oper(MDCT2_L_WIN2, tmp16_norm_shift, ykr, ykr, &shl);

    /* compute overall normalization factor */
    *norm_shift = add(*norm_shift, tmp16_norm_shift);

    /* update memory */
    mov16(MDCT2_L_WIN2, input, mem);


    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/

    return;
}                               /* END MDCT */

void PCMSWB_TDAC_inv_mdct (
                           Word16 * xr,         /* (o):   output samples                     */
                           Word16 * ykq,        /* (i):   MDCT coefficients                  */
                           Word16 * ycim1,      /* (i):   previous MDCT memory               */
                           Word16   norm_shift, /* (i):   norm_shift value defined by coder  */
                           Word16 * norm_pre,   /* (i/o): norm_shift value of previous frame */
                           Word16   loss_flag,  /* (i):   packet-loss flag                   */
                           Word16 * cur_save    /* (i/o): signal saving buffer               */
                           )
{
    Word32   ACC0;

    Word16   ycr[MDCT2_L_WIN4];
    Word16   yci[MDCT2_L_WIN4];
    Word16   sig_cur[MDCT2_L_WIN2];
    Word16   sig_next[MDCT2_L_WIN2];

    Word16   *ptr_yci;
    Word16   *ptr_ycr;
    Word16   *ptr1;
    Word16   *ptr2;
    Word16   *ptr1_next;
    Word16   *ptr2_next;
    const Word16 *ptr_h;         /* Pointer on window */

    Word16   k,n;
    Word16   tmp16;

    Word32   ACC1;
    Word16   n1;

    const Word16   *ptr_wsin, *ptr_wcos;

    /*****************************/
#ifdef DYN_RAM_CNT
    {
        UWord32 ssize;
        ssize = (UWord32) (9 * SIZE_Ptr);
        ssize += (UWord32) ((2 * MDCT2_L_WIN4 + 2 * MDCT2_L_WIN2 + 4) * SIZE_Word16);
        ssize += (UWord32) (2 * SIZE_Word32);
        DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    /*****************************/  

    /* Higher-band frame erasure concealment (FERC) in time domain */
    IF (loss_flag != 0)
    {
        n1 = sub(*norm_pre, 6);
        ptr_h = MDCT_h_swb + MDCT2_L_WIN2;
        FOR (k = 0; k < MDCT2_L_WIN2; k++)
        {
            cur_save[k] = mult_r ((Word16)28672/*ATT_FEC_COEF*/, cur_save[k]); move16();

            ACC0 = L_mult0(cur_save[k], MDCT_h_swb[k]);
            ACC0 = L_mac0(ACC0, ycim1[k], *(--ptr_h));
            ACC0  = L_shr(ACC0, n1);
            xr[k] = round_fx(ACC0); move16();

            ycim1[k] = mult_r ((Word16)28672/*ATT_FEC_COEF*/, ycim1[k]); move16();
        }

        /*****************************/
#ifdef DYN_RAM_CNT
        DYN_RAM_POP();
#endif
        /*****************************/

        return;
    }

    /*******************************************************************************/
    /* Inverse MDCT computation                                                    */
    /*******************************************************************************/

    /* 1 --> Input rotation = Product by wetrm1 */

    ptr1 = ykq;
    ptr2 = ykq + (MDCT2_L_WIN2 - 1);   
    ptr_yci = yci;
    ptr_ycr = ycr;

    FOR (k = 0; k < MDCT2_L_WIN4; k++)
    {
        ACC0 = L_msu0(0, *ptr2, MDCT_wetrm1_swb[k]);

        ACC0 = L_msu0(ACC0, *ptr1, MDCT_wetim1_swb[k]);
        *ptr_ycr++ = round_fx(ACC0); move16();


        ACC0 = L_mult0(*ptr1++, MDCT_wetrm1_swb[k]);
        ACC0 = L_msu0(ACC0, *ptr2--, MDCT_wetim1_swb[k]);
        *ptr_yci++ = round_fx(ACC0); move16();
        ptr1++;
        ptr2--;
    }

    /* 2 --> Forward FFT : size = 20 */
    cfft(ycr, yci, 1);

    /* 3 --> Output rotation : product by a complex exponent */

    ptr_yci = yci;
    ptr_ycr = ycr;

    ptr_wsin = MDCT_wsin_swb;                      move16();
    ptr_wcos = MDCT_wsin_swb + MDCT2_L_WIN4; move16();   

    FOR (k = 0; k < MDCT2_L_WIN4; k++)
    {
        tmp16 = *ptr_ycr;

        ACC0 = L_mult0(tmp16, *ptr_wcos);
        ACC0 = L_mac0(ACC0, yci[k], *ptr_wsin);
        *ptr_ycr++ = round_fx(ACC0); move16();

        ACC0 = L_mult0(yci[k], *ptr_wcos);
        ACC0 = L_msu0(ACC0, tmp16, *ptr_wsin);
        *ptr_yci++ = round_fx(ACC0); move16();

        ptr_wsin++;
        ptr_wcos--;
    }

    /* 4 --> Overlap and windowing (in one step) - equivalent to complex product */

    ptr1 = sig_cur;
    ptr2 = sig_cur + MDCT2_L_WIN2 - 1;   
    ptr1_next = sig_next;
    ptr2_next = sig_next + MDCT2_L_WIN2 - 1;   

    FOR (k = 0; k < MDCT2_L_WIN4; k++)
    {
        *ptr1++ = ycr[k];              move16();
        *ptr2--      = negate(ycr[k]); move16();
        *ptr1_next++ = yci[k];         move16();
        *ptr2_next-- = yci[k];         move16();

        ptr1++;
        ptr1_next++;
        ptr2--;
        ptr2_next--;
    }

    n = sub(norm_shift, 6);
    n1 = sub(*norm_pre, 6);
    ptr_h = MDCT_h_swb + MDCT2_L_WIN2;
    FOR (k = 0; k < MDCT2_L_WIN2; k++)
    {
        ACC0  = L_mult0(sig_cur[k], MDCT_h_swb[k]);
        ACC0  = L_shr(ACC0, n);
        ACC1  = L_mult0(ycim1[k], *(--ptr_h));
        ACC1  = L_shr(ACC1, n1);
        ACC0  = L_add(ACC0, ACC1);
        xr[k] = round_fx(ACC0);
        ycim1[k] = sig_next[k];        move16();
    }

    /* Save sig_cur for FERC */
    mov16(MDCT2_L_WIN2, sig_cur, cur_save);
    *norm_pre = norm_shift;  move16();

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/

    return;
}

/*--------------------------------------------------------------------------*
*  Function  cfft()                                                        *
*                                                                          *
*  Complex Good-Thomas fast Fourier transform                              *
*--------------------------------------------------------------------------*/

static void cfft(
                 Word16 * x1,   /* (i/o) real part of data                 */
                 Word16 * x2,   /* (i/o) imaginary part of data            */
                 Word16   sign  /* (i) flag to select FFT (1) or IFFT (-1) */
                 )
{
    Word32    ACC0;                 /* first ACC */
    Word32    ACC1;                 /* second ACC */

    Word16    tab_x1[MDCT2_NP * MDCT2_NPP];   
    Word16    tab_x2[MDCT2_NP * MDCT2_NPP];   
    Word16    rx1[MDCT2_NP];
    Word16    rx2[MDCT2_NP];

    Word16   *ptr_x1;               /* Pointer on tab_x1 */
    Word16   *ptr_x2;               /* Pointer on tab_x2 */
    Word16   *ptr0_x1;              /* Pointer on tab_x1 for DFT step */
    Word16   *ptr0_x2;              /* Pointer on tab_x2 for DFT step */
    const Word16 *ptr_cos;          /* Pointer on cos table */
    const Word16 *ptr_sin;          /* Pointer on sin table */
    const Word16 *ptr_cos_Overflow; /* Pointer on cos table (case of Overflow) */
    const Word16 *ptr_sin_Overflow; /* Pointer on sin table (case of Overflow) */
    const Word16 *ptr_map;          /* Pointer on mapping indice (input and output) */

    Word16    ip, ipp, i, j;
    Word16    x1_tmp;
    Word16    x2_tmp;
    Word16    exp_n2;
    Word16    n1;
    Word16    n2;                 /* size of sub array */
    Word16    n3;
    Word16    p;                  /* size of the butterfly */
    Word16    q;
    Word16    inkw;
    Word16    w1;
    Word16    w2;
    Word16    rix;
    Word16    cix;
    Word16    rjx;
    Word16    cjx;

    /*****************************/
#ifdef DYN_RAM_CNT
    {
        UWord32 ssize;
        ssize = (UWord32) (9 * SIZE_Ptr);
        ssize += (UWord32) ((2 * MDCT2_NP * MDCT2_NPP + 2 * MDCT2_NP + 19) * SIZE_Word16);
        ssize += (UWord32) (2 * SIZE_Word32);
        DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    /*****************************/

    /********************************************************************************
    * Re-indexing (mapping of input indices)                                       *
    ********************************************************************************/

    ptr_x1 = tab_x1;
    ptr_x2 = tab_x2;
    ptr_map = MDCT_tab_map_swb;

    FOR (ip = 0; ip < MDCT2_NP * MDCT2_NPP; ip++)
    {   
        *ptr_x1++ = x1[(Word16) * ptr_map];         move16();
        *ptr_x2++ = x2[(Word16) * ptr_map++];         move16();
    }

    /*******************************************************************************/

    ptr_x1 = tab_x1;
    ptr_x2 = tab_x2;

    FOR (ip = 0; ip < MDCT2_NP; ip++)
    {
        x1_tmp = ptr_x1[MDCT_tab_rev_ipp_swb[0]];      move16(); /* swap value ptr_x1[i] and ptr_x1[ipp] */
        x2_tmp = ptr_x2[MDCT_tab_rev_ipp_swb[0]];      move16(); /* swap value ptr_x2[i] and ptr_x2[ipp] */
        ptr_x1[MDCT_tab_rev_ipp_swb[0]] = ptr_x1[MDCT_tab_rev_i_swb[0]];   move16();
        ptr_x2[MDCT_tab_rev_ipp_swb[0]] = ptr_x2[MDCT_tab_rev_i_swb[0]];   move16();
        ptr_x1[MDCT_tab_rev_i_swb[0]] = x1_tmp;        move16();
        ptr_x2[MDCT_tab_rev_i_swb[0]] = x2_tmp;        move16();

        x1_tmp = ptr_x1[MDCT_tab_rev_ipp_swb[1]];      move16(); /* swap value ptr_x1[i] and ptr_x1[ipp] */
        x2_tmp = ptr_x2[MDCT_tab_rev_ipp_swb[1]];      move16(); /* swap value ptr_x2[i] and ptr_x2[ipp] */
        ptr_x1[MDCT_tab_rev_ipp_swb[1]] = ptr_x1[MDCT_tab_rev_i_swb[1]];   move16();
        ptr_x2[MDCT_tab_rev_ipp_swb[1]] = ptr_x2[MDCT_tab_rev_i_swb[1]];   move16();
        ptr_x1[MDCT_tab_rev_i_swb[1]] = x1_tmp;        move16();
        ptr_x2[MDCT_tab_rev_i_swb[1]] = x2_tmp;        move16();

        ptr_x1 += MDCT2_NPP;   
        ptr_x2 += MDCT2_NPP;   
    }

    /*******************************************************************************
    * n1 size of butterfly                                                        *
    *******************************************************************************/

    ptr_x1 = tab_x1;
    ptr_x2 = tab_x2;

    IF(sign > 0)
    {
        FOR (ip = 0; ip < MDCT2_NP; ip++)
        {
            FOR (exp_n2 = 0; exp_n2 <= MDCT2_EXP_NPP; exp_n2++)
            {
                n2 = shl(1, exp_n2);
                n1 = shr(n2, 1);
                n3 = sub(MDCT2_EXP_NPP, exp_n2);
                inkw = 0; move16();
                q = shl(1, n3);

                FOR (p = 0; p < n1; p++)
                {
                    /* get twiddle factor in arrays rw1 and rw2 */
                    w1 = MDCT_rw1_tbl_swb[inkw]; move16();
                    w2 = MDCT_rw2_tbl_swb_n[inkw]; move16();
                    inkw = add( inkw, q); /* q is constant inside the loop */

                    FOR (i = p; i < MDCT2_NPP; i += n2)
                    {
                        /* select item p in array p */
                        j = add(i, n1);

                        /* butterfly on x[i] and x[j] */
                        rix = ptr_x1[i]; move16();
                        cix = ptr_x2[i]; move16();

                        /* twiddle factor */
                        ACC0 = L_mult(w1, ptr_x1[j]);
                        rjx = msu_r(ACC0, w2, ptr_x2[j]);
                        ACC0 = L_mult(w2, ptr_x1[j]);

                        cjx = mac_r(ACC0, w1, ptr_x2[j]);

                        ptr_x1[i] = add(rix, rjx); move16();
                        ptr_x2[i] = add(cix, cjx); move16();
                        ptr_x1[j] = sub(rix, rjx); move16();
                        ptr_x2[j] = sub(cix, cjx); move16();
                    }
                }
            }                           /* end while */
            ptr_x1 += MDCT2_NPP;   
            ptr_x2 += MDCT2_NPP;   
        }                             /* end for ip */
    }
    ELSE
    {
        FOR (ip = 0; ip < MDCT2_NP; ip++)
        {
            FOR (exp_n2 = 0; exp_n2 <= MDCT2_EXP_NPP; exp_n2++)
            {
                n2 = shl(1, exp_n2);
                n1 = shr(n2, 1);
                n3 = sub(MDCT2_EXP_NPP, exp_n2);
                inkw = 0; move16();
                q = shl(1, n3);

                FOR (p = 0; p < n1; p++)
                {
                    /* get twiddle factor in arrays rw1 and rw2 */
                    w1 = MDCT_rw1_tbl_swb[inkw]; move16();
                    w2 = MDCT_rw2_tbl_swb[inkw]; move16();
                    inkw = add( inkw, q); /* q is constant inside the loop */

                    FOR (i = p; i < MDCT2_NPP; i += n2)
                    {
                        /* select item p in array p */
                        j = add(i, n1);

                        /* butterfly on x[i] and x[j] */
                        rix = shr(ptr_x1[i], 1);
                        cix = shr(ptr_x2[i], 1);

                        /* twiddle factor */
                        ACC0 = L_mult0(w1, ptr_x1[j]);
                        ACC0 = L_msu0(ACC0, w2, ptr_x2[j]);
                        rjx = round_fx(ACC0);

                        ACC0 = L_mult0(w2, ptr_x1[j]);
                        ACC0 = L_mac0(ACC0, w1, ptr_x2[j]);
                        cjx = round_fx(ACC0);

                        ptr_x1[i] = add(rix, rjx); move16();
                        ptr_x2[i] = add(cix, cjx); move16();
                        ptr_x1[j] = sub(rix, rjx); move16();
                        ptr_x2[j] = sub(cix, cjx); move16();
                    }
                }
            }                           /* end while */
            ptr_x1 += MDCT2_NPP;   
            ptr_x2 += MDCT2_NPP;   
        }                             /* end for ip */
    }

    /**************************************************************************/

    ptr0_x1 = tab_x1;
    ptr0_x2 = tab_x2;

    IF(sign > 0)
    {
        FOR (ipp = 0; ipp < MDCT2_NPP; ipp++)
        {
            ptr_x1 = ptr0_x1;
            ptr_x2 = ptr0_x2;

            FOR (ip = 0; ip < MDCT2_NP; ip++)
            {
                rx1[ip] = *ptr_x1; move16();
                rx2[ip] = *ptr_x2; move16();

                ptr_x1 += MDCT2_NPP;   
                ptr_x2 += MDCT2_NPP;   
            }

            ptr_x1 = ptr0_x1++;
            ptr_x2 = ptr0_x2++;

            ptr_cos = MDCT_xcos_swb;
            ptr_sin = MDCT_xsin_swb;


            FOR (ip = 0; ip < MDCT2_NP; ip++)
            {

                /* Set Overflow to 0 to test it after radix 5 with Q15 sin & cos coef */
                /* keep pointer's position on cos & sin tables */

                Overflow = 0; move16();
                ptr_cos_Overflow = ptr_cos;
                ptr_sin_Overflow = ptr_sin;

                ACC0 = L_mac(0, rx1[0], (*ptr_cos));
                ACC0 = L_msu(ACC0, rx2[0], (*ptr_sin));

                ACC1 = L_mac(0, rx2[0], (*ptr_cos++));
                ACC1 = L_mac(ACC1, rx1[0], (*ptr_sin++));

                FOR (i = 1; i < MDCT2_NP; i++)
                {
                    ACC0 = L_mac(ACC0, rx1[i], (*ptr_cos));
                    ACC0 = L_msu(ACC0, rx2[i], (*ptr_sin));

                    ACC1 = L_mac(ACC1, rx2[i], (*ptr_cos++));
                    ACC1 = L_mac(ACC1, rx1[i], (*ptr_sin++));
                }

                ACC0 = L_shr(ACC0, 1);
                ACC1 = L_shr(ACC1, 1);

                /* Overflow in Radix 5 --> use cos and sin coef in Q14 */
                IF (Overflow)
                {
                    ptr_cos = ptr_cos_Overflow;
                    ptr_sin = ptr_sin_Overflow;

                    ACC0 = L_mac(0, rx1[0], shr((*ptr_cos), 1));
                    ACC0 = L_msu(ACC0, rx2[0], shr((*ptr_sin), 1));

                    ACC1 = L_mac(0, rx2[0], shr((*ptr_cos++), 1));
                    ACC1 = L_mac(ACC1, rx1[0], shr((*ptr_sin++), 1));

                    FOR (i = 1; i < MDCT2_NP; i++)
                    {
                        ACC0 = L_mac(ACC0, rx1[i], shr((*ptr_cos), 1));
                        ACC0 = L_msu(ACC0, rx2[i], shr((*ptr_sin), 1));

                        ACC1 = L_mac(ACC1, rx2[i], shr((*ptr_cos++), 1));
                        ACC1 = L_mac(ACC1, rx1[i], shr((*ptr_sin++), 1));
                    }
                }


                *ptr_x1 = round_fx(ACC0); move16();
                *ptr_x2 = round_fx(ACC1); move16();

                ptr_x1 += MDCT2_NPP;   
                ptr_x2 += MDCT2_NPP;   
            }
        }                             /* end for ipp */
    }
    ELSE
    {
        FOR (ipp = 0; ipp < MDCT2_NPP; ipp++)
        {
            ptr_x1 = ptr0_x1;
            ptr_x2 = ptr0_x2;

            FOR (ip = 0; ip < MDCT2_NP; ip++)
            {
                rx1[ip] = *ptr_x1; move16();
                rx2[ip] = *ptr_x2; move16();

                ptr_x1 += MDCT2_NPP;   
                ptr_x2 += MDCT2_NPP;   
            }

            ptr_x1 = ptr0_x1++;
            ptr_x2 = ptr0_x2++;

            ptr_cos = MDCT_xcos_swb;
            ptr_sin = MDCT_xsin_swb;


            FOR (ip = 0; ip < MDCT2_NP; ip++)
            {

                /* Set Overflow to 0 to test it after radix 5 with Q15 sin & cos coef */
                /* keep pointer's position on cos & sin tables */

                Overflow = (Word16) 0; move16();
                ptr_cos_Overflow = ptr_cos;   
                ptr_sin_Overflow = ptr_sin;   

                ACC0 = L_mac(0, rx1[0], (*ptr_cos));
                ACC0 = L_mac(ACC0, rx2[0], (*ptr_sin));

                ACC1 = L_mac(0, rx2[0], (*ptr_cos++));
                ACC1 = L_msu(ACC1, rx1[0], (*ptr_sin++));
                FOR (i = 1; i < MDCT2_NP; i++)
                {
                    ACC0 = L_mac(ACC0, rx1[i], (*ptr_cos));
                    ACC0 = L_mac(ACC0, rx2[i], (*ptr_sin));

                    ACC1 = L_mac(ACC1, rx2[i], (*ptr_cos++));
                    ACC1 = L_msu(ACC1, rx1[i], (*ptr_sin++));
                }

                ACC0 = L_shr(ACC0, 1);
                ACC1 = L_shr(ACC1, 1);

                /* 0verflow in Radix 5 --> use cos and sin coef in Q14 */
                IF (Overflow)
                {
                    ptr_cos = ptr_cos_Overflow;
                    ptr_sin = ptr_sin_Overflow;

                    ACC0 = L_mac(0, rx1[0], shr((*ptr_cos), 1));
                    ACC0 = L_mac(ACC0, rx2[0], shr((*ptr_sin), 1));

                    ACC1 = L_mac(0, rx2[0], shr((*ptr_cos++), 1));
                    ACC1 = L_msu(ACC1, rx1[0], shr((*ptr_sin++), 1));
                    FOR (i = 1; i < MDCT2_NP; i++)
                    {
                        ACC0 = L_mac(ACC0, rx1[i], shr((*ptr_cos), 1));
                        ACC0 = L_mac(ACC0, rx2[i], shr((*ptr_sin), 1));

                        ACC1 = L_mac(ACC1, rx2[i], shr((*ptr_cos++), 1));
                        ACC1 = L_msu(ACC1, rx1[i], shr((*ptr_sin++), 1));
                    }
                }

                *ptr_x1 = round_fx(ACC0); move16();
                *ptr_x2 = round_fx(ACC1); move16();

                ptr_x1 += MDCT2_NPP;   
                ptr_x2 += MDCT2_NPP;   
            }                         /* end for ip */
        }                             /* end for ipp */
    }

    /***************************************************************************
    * mapping for the output indices                                          *
    ***************************************************************************/
    ptr_x1 = tab_x1;
    ptr_x2 = tab_x2;
    ptr_map = MDCT_tab_map2_swb;

    FOR (ip = 0; ip < MDCT2_NP * MDCT2_NPP; ip++)
    {
        x1[(Word16) * ptr_map]   = *ptr_x1++;        move16();
        x2[(Word16) * ptr_map++] = *ptr_x2++;        move16();
    }

    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/

    return;
}
