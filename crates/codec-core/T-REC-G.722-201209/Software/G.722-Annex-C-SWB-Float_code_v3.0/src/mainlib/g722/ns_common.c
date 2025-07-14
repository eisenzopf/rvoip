/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "ns.h"
#include "hsb_enh.h"

Float fl_noise_shaper(Float *A, Float in, Float *mem)
{
  int j;
  Float out;


  /* Calculation of the weighted error signal */
  out = A[0]* in; 
  for(j=0; j<ORD_M; j++) {
    out += A[j+1]* (*mem--);
  }
  out = Floor(out);
  return(out); 
}


