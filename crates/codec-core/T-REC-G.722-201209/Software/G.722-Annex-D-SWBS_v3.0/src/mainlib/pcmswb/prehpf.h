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
void  highpass_1tap_iir (Word16, Word16, Word16*, Word16*, void*);
#ifdef LAYER_STEREO
void  highpass_1tap_iir_stereo (Word16, Word16, Word16*, Word16*, void*);
#endif

#endif  /* FPREHPF_H */
