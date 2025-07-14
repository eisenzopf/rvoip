/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include "typedef.h"

/* For functions: hsbupd() and lsbupd() */
Word16 inv_frm_size[];

/* For function: quantl_toupdatescaling_logscl() */
Word16 wlil4rilil[];
extern Word16 q4[];

/* For functions: hsbdec(), plc_hsbdec() */
extern Word16 NGFAEOFFSET_P1[];

/* LPC analysis windowing */
extern Word16   win[];

/* spectral smooth technique */
extern Word16   sstwin_h[];
extern Word16   sstwin_l[];

/* bandwidth expansion */
extern Word16   bwel[];

/* spectral weighting */
extern Word16   STWAL[];

extern Word16 nbphtab[];
extern Word16 nbpltab[];

extern Word16 ola3[];
extern Word16 ola4[];
extern Word16 ola5[];
extern Word16 ola6[];
extern Word16 ola7[];
extern Word16 ola8[];

/* coarse pitch search */
extern  Word16  bdf[];
extern  Word16  x[];
extern  Word16  x2[];
extern  Word16  invk[];
extern  Word16  MPTH[];

extern	Word16	pp9cb[];

extern	Word16	olaup[];
extern	Word16 	oladown[];
extern	Word16  olaug[];
extern	Word16  oladg[];

extern	Word16	wn[];
extern	Word16	gawd[];

extern	Word16	div_n[];

extern void Log2(Word32, Word16*, Word16 *);
extern Word32 Pow2(Word16, Word16);

extern Word16	tablog[];
extern Word16	tabpow[];


/* The following are used to control the internal Q-value in apfilter_shift */
#define apQ         6
#define ap_shift    (16-apQ)

