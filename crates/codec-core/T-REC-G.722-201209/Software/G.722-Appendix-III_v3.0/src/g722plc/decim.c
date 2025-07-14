/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include "typedef.h"
#include "g722plc.h"
#include "table.h"
#include "stl.h"
#if (DMEM)
#include "memutil.h"
#endif
#include "utility.h"

/*-----------------------------------------------------------------------------
 * Function: decim()
 *
 * Description: DECIMate the weighted speech signal from 16 kHz to 2 kHz.
 *              The 1 kHz anti-aliasing low-pass filtering is performed with
 *              a 60th-order minimum-phase FIR filter. The decimation filtering  
 *              operation is performed only once every 8 samples -- only when 
 *              an output 2 kHz signal sample is needed.
 *
 * Inputs:  *xw   - pointer to 16 kHz weighted speech buffer
 *          *xwd  - pointer to 2 kHz decimated weighted speech buffer
 *          *cstate - data structure containing the states of G.722 PLC
 * Outputs: *xwd  - pointer to 2 kHz decimated weighted speech buffer
 *---------------------------------------------------------------------------*/
void decim(
Word16 	*xw,				
Word16	*xwd,
struct WB_PLC_State *cstate)
{
   Word32    a0, a1;
   Word32    *lp0;
   Word16    exp, new_exp;
   Word16    *fp0;
   Word16    exp0, exp1;
   Word16    i, j;
   Word16    *fp;
#if (DMEM)
   Word32 *lxwd;
#else
   Word32 lxwd[FRSZD];
#endif

   a1 = L_mult(1,1);

   /* load decimation filter memory to beginning part of xw[] array */ 
   W16copy(xw, cstate->dfm, DFO);

#if (DMEM)
   lxwd = allocWord32(0, FRSZD-1);
#endif

   /* low-pass filtering of xw[] every 8th sample, save output to lxwd[] */ 
   lp0 = lxwd;
   FOR(i=7;i<FRSZ;i=i+8)
   {
      fp = &xw[DFO+i];
      a0 = L_mult0(bdf[0],*fp--);
      FOR (j=0;j<DFO-1;j++) 
         a0 = L_mac0(a0,bdf[j+1],*fp--);
      *lp0++=a0;
#if WMOPS
      move32();
#endif
      a0 = L_abs(a0);
      a1 = L_max(a0,a1);
   }
   /* update decimation filter state memory */
   W16copy(cstate->dfm, xw+FRSZ, DFO);

   /* setup local xwd[] */
   lp0 = lxwd;
   new_exp = sub(norm_l(a1), 3);
   exp = sub(cstate->xwd_exp, new_exp);
 
   if (exp < 0)
   {
      new_exp = cstate->xwd_exp;
#if WMOPS
      move16();
#endif 
   }
   exp = s_max(exp, 0);

   FOR (i=0;i<XDOFF;i++) 
   {
      xwd[i] = shr(cstate->xwd[i], exp);
#ifdef WMOPS
      move16();
#endif
   }

   fp0 = &xwd[XDOFF];
   FOR (i=0;i<FRSZD;i++) 
   {
      fp0[i] = round(L_shl(lp0[i],new_exp));
#ifdef WMOPS
      move16();
#endif
   }
  
#if (DMEM)  
   /* memory deallocation */
   deallocWord32(lxwd, 0, FRSZD-1);
#endif
  
   /* update xwd() memory */
   exp0 = 1;
#if WMOPS
   move16();
#endif
   FOR (i=0;i<XDOFF;i++) 
   {
      exp1 = abs_s(xwd[FRSZD+i]);
      exp0 = s_max(exp0, exp1);
   }
   exp0 = sub(norm_s(exp0),3);
   exp = sub(exp0, exp);

   FOR (i=0;i<XDOFF-FRSZD;i++) 
   {
      cstate->xwd[i] = shl(cstate->xwd[i+FRSZD], exp);
#ifdef WMOPS
      move16();
#endif
   }
   FOR (;i<XDOFF;i++) 
   {
      cstate->xwd[i] = shl(xwd[FRSZD+i],exp0);
#ifdef WMOPS
      move16();
#endif
   }
  
   cstate->xwd_exp = add(new_exp, exp0);
#ifdef WMOPS
   move16();
#endif  
}
