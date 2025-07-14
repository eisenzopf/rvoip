/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: prehpf.h
 *  Function: Header of pre-processing 1-tap high-pass filtering
 *------------------------------------------------------------------------
 */

#ifndef FPREHPF_H
#define FPREHPF_H

void  *highpass_1tap_iir_const (void);
void  highpass_1tap_iir_dest (void*);
void  highpass_1tap_iir_reset (void*);
void  highpass_1tap_iir (Short, Short, Short*, Float*, void*);

#endif  /* FPREHPF_H */
