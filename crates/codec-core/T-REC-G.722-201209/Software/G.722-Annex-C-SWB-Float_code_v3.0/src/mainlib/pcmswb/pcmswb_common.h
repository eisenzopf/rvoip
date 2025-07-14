/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  Function: Common definitions for all module files
 *------------------------------------------------------------------------
 */

#ifndef COMMON_DEFS_H
#define COMMON_DEFS_H

#include <stdio.h>
#include <stdlib.h>
#include "floatutil.h"
#include "pcmswb.h"

#define L_FRAME_NB  NSamplesPerFrame08k  /* Number of samples in  8 kHz */
#define L_FRAME_WB  NSamplesPerFrame16k  /* Number of samples in 16 kHz */
#define L_FRAME_SWB NSamplesPerFrame32k  /* Number of samples in 32 kHz */

#endif
