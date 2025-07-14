/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: qmfilt.h
 *  Function: Header of quadrature mirror filter (QMF)
 *------------------------------------------------------------------------
 */

#ifndef QMFILT_H
#define QMFILT_H

#define NTAP_QMF_WB  32
#define QMF_DELAY_WB (NTAP_QMF_WB-2)

#define NTAP_QMF_G722    24
#define QMF_DELAY_G722   (NTAP_QMF_G722-2)
#define NTAP_QMF_SWB     (NTAP_QMF_WB)
#define QMF_DELAY_SWB    (QMF_DELAY_WB)

void* QMFilt_const(Word16 ntap, const Word16 *qmf0, const Word16 *qmf1);
void  QMFilt_dest(void *ptr);
void  QMFilt_reset(void *ptr);
void  QMFilt_ana(Word16 n, Word16 *insig, Word16 *lsig, Word16 *hsig, void *prt);
void  QMFilt_syn(Word16 n, Word16 *lsig, Word16 *hsig, Word16 *outsig, void *ptr);

/* QMF coefficients tables */
extern const Word16 sSWBQmf0[NTAP_QMF_SWB/2]; /* Q15 */
extern const Word16 sSWBQmf1[NTAP_QMF_SWB/2]; /* Q15 */

#endif
