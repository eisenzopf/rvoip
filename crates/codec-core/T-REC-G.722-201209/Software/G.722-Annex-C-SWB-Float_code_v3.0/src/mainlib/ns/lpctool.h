/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: lpctool.h
 *  Function: Header of linear prediction tools
 *------------------------------------------------------------------------
 */

#ifndef LPCTOOL_H
#define LPCTOOL_H


void  fl_Levinson(Float R[], Float rc[], Short *stable, Short ord, Float *a);
void  fl_Lag_window(Float *R, const Float *W, Short ord);
void  fl_Weight_a(Float a[], Float ap[], Float gamma, Short m);

#endif
