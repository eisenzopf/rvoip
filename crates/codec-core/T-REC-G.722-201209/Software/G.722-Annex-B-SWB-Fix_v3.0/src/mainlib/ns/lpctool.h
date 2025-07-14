/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: lpctool.h
 *  Function: Header of linear prediction tools
 *------------------------------------------------------------------------
 */

#ifndef LPCTOOL_H
#define LPCTOOL_H

void  Levinson(Word16 R_h[], Word16 R_l[], Word16 rc[], Word16 * stable, Word16 ord, Word16 * a);
void  Lag_window(Word16 * R_h, Word16 * R_l, const Word16 * W_h, const Word16 * W_l, Word16 ord);
void  Autocorr(Word16 x[], const Word16 win[], Word16 r_h[], Word16 r_l[], Word16 ord, Word16 len);
void  Weight_a(Word16 a[], Word16 ap[], Word16 gamma, Word16 m);

#endif
