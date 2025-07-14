/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "ns.h"
#include "hsb_enh.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

Word16 noise_shaper(Word16 *A, Word16 in, Word16 *mem)
{
  Word16 j;
  Word16 out;
  Word32  lval;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((1) * SIZE_Word32);
    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  /* Calculation of the weighted error signal */
  lval = L_mult(A[0], in);    /* put sigin in 32-bits in Q12 (A[0]=1.0) */
  FOR (j=0; j<ORD_M; j++) {
    lval = L_mac(lval, A[j+1], *mem--);
  }
  out = extract_h_L_shl(lval, 3);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return(out); 
}



