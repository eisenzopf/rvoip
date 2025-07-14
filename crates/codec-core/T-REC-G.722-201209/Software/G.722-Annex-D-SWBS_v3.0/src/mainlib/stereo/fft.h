/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: fft.h
 *  Function: Header of FFT for stereo
 *------------------------------------------------------------------------
 */

#ifndef FFTSTEREO_H
#define FFTSTEREO_H

#ifdef LAYER_STEREO
extern Word16 twiddleRe[64],twiddleIm[64];

void fixDoRFFTx(Word16 x[], Word16 *x_q);
void fixDoRiFFTx(Word16 x[], Word16 *x_q);
#endif /*LAYER_STEREO*/

#endif  /* FFTSTEREO_H */
