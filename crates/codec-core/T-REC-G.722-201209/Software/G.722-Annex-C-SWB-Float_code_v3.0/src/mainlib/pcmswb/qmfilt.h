/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: qmfilt.h
 *  Function: Header of quadrature mirror filter (QMF)
 *------------------------------------------------------------------------
 */

#ifndef QMFILT_H
#define QMFILT_H

void* QMFilt_const(int ntap, const Float *qmf0, const Float *qmf1);
void  QMFilt_dest(void *ptr);
void  QMFilt_reset(void *ptr);
void  QMFilt_syn(int n, Float *lsig, Float *hsig, Float *outsig, void *ptr);
void  QMFilt_ana(int n, Float *insig, Float *lsig, Float *hsig, void *prt);

/* QMF coefficients tables */
extern const Float fSWBQmf0[NTAP_QMF_SWB/2];
extern const Float fSWBQmf1[NTAP_QMF_SWB/2];
#endif
