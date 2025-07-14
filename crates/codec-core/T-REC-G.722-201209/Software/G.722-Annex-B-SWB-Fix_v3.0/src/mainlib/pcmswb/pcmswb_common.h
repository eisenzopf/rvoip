/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  Function: Common definitions for all module files
 *------------------------------------------------------------------------
 */

#ifndef COMMON_DEFS_H
#define COMMON_DEFS_H

#include "dsputil.h"
#include "pcmswb.h"

#define L_FRAME_NB  NSamplesPerFrame08k  /* Number of samples in  8 kHz */
#define L_FRAME_WB  NSamplesPerFrame16k  /* Number of samples in 16 kHz */
#define L_FRAME_SWB NSamplesPerFrame32k  /* Number of samples in 32 kHz */

#endif
