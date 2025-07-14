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

#define	xqoff	(XQOFF+FRSZ)

/*-----------------------------------------------------------------------------
 * Function: prfn()
 *
 * Description: Pitch period ReFiNement.
 *              This function performs a refinement search around the coarse 
 *              pitch period with the 16 kHz time resolution based on the 
 *              16 kHz G.722/PLC output speech signal.
 *
 * Inputs:  *xq - pointer to 16 kHz output speech buffer
 *          cpp - coarse pitch period in 16 kHz time resolution
 * Outputs: (return value of the function) - refined pitch period 
 *          *ptfe - Pitch Tap for Frame Erasure (scaling factor for PWE)
 *          *cormax - CORrelation MAXimum during pitch refinement search
 *          *energymax32 - ENERGY MAXimum during pitch refinement search
 *          *ppt - Pitch Predictor Tap for calculating long-term ringing
 *          *wsz - adaptive Window SiZe chosen for pitch refinement
 *          *sflag - Shift FLAG for xq[]
 *---------------------------------------------------------------------------*/
Word16  prfn(
Word16  *ptfe,      /* (o) Q14 pitch tap */
Word32  *cormax,
Word32  *energymax32,
Word16  *ppt,
Word16  *wsz,
Word16  *sflag,
Word16  *xq,        /* (i) quantized signal from last sub-frame */
Word16  cpp)        /* (i) pitch period from last subframe */
{
	Word32	a0, a1, amy;
	Word16	*xqp, *pp;
	Word16   *x, *y, *fp0, *fp1;
	Word16	s, t, lasts;
	Word16	cor2max, cor2max_exp;
	Word16	energymax, energymax_exp;
	Word16	ener, ener_exp;
	Word16	cor2, cor2_exp;
	Word32	energy, cor;
	Word32   cormax_local, energymax32_local;
	Word16 	lb, ub, ppfe, tt;
	Word16	i, k;
   Word16   shift;

#if (DMEM)
	Word16 *xqbuf;
#else
	Word16 xqbuf[xqoff];
#endif

#if (DMEM)
	/* memory allocation */
	xqbuf = allocWord16(0, xqoff-1);
#endif

	*wsz = s_min(cpp,WML); 
	lb = sub(cpp, 3);
	lb = s_max(lb, MINPP); /* lower bound of pitch period search range */ 
	ub = add(cpp, 3);
	ub = s_min(ub, MAXPP); /* upper bound of pitch period search range */

	/* CHECK IF THE SIGNAL SEGMENT CAUSES OVERFLOW */
	energy = 0;
#if WMOPS
   move16();
#endif
	fp1 = &xq[sub(sub(xqoff, (*wsz)), lb)];
	FOR (k=0;k<*wsz;k++) 
   {
	    t = shr(*fp1++, 3);
	    energy = L_mac0(energy,t,t);
	}
   shift = norm_l(energy);
   shift = sub(6, shift);
   IF (shift > 0)
   {
      /* memory allocation */
      xqp = xqbuf;
      shift = shr(add(shift, 1), 1);
      *sflag = shift;
#if WMOPS
      move16();
#endif
      FOR (i=0;i<xqoff;i++)
      {
         xqbuf[i] = shr(xq[i],shift);
#ifdef WMOPS
         move16();
#endif
      }
	} ELSE {
	    xqp = xq;
		 *sflag = 0;
#if WMOPS
       move16();
#endif
	}

    /* HANDLE THE FIRST CANDIDATE OUT OF THE LOOP */
	x = &xqp[sub(xqoff, (*wsz))];				/* target vector for the search */
	fp0 = x;
	fp1 = &xqp[sub(sub(xqoff, (*wsz)),lb)];
   energy=L_mult0(*fp1,*fp1);
   cor = L_mult0(*fp0++,*fp1++);
	FOR (k=1;k<*wsz;k++) {
		energy = L_mac0(energy,*fp1,*fp1);
		cor = L_mac0(cor,*fp0++,*fp1++);
	}

	cor = L_max(cor, -1);
  	cormax_local=L_max(cor,cor);
	cor2max_exp = norm_l(cor);
	s = extract_h(L_shl(cor, cor2max_exp));
	cor2max_exp = shl(cor2max_exp, 1);
	cor2max = extract_h(L_mult(s, s));
	energymax32_local = L_max(energy,energy);
	energymax_exp = norm_l(energy);
	energymax = extract_h(L_shl(energy, energymax_exp));
  	ppfe=lb;
#if WMOPS
	move16();
#endif

    /* NOW SEARCH THE REST OF CANDIDATES FOR MAXIMUM PITCH PREDICTION GAIN */
   pp = &xqp[sub(sub(xqoff, (*wsz)), (Word16)(lb+1))];
	FOR (k=lb+1;k<=ub;k++) {
		fp0 = x;
		fp1 = pp--;
		lasts = *fp1;
      cor = L_mult0(*fp0++,*fp1++);
#if WMOPS
      move16();
#endif
		FOR (i=1;i<(*wsz);i++) 
      {
			cor = L_mac0(cor,*fp0++,*fp1++);
		}

		energy = L_msu0(energy,*fp1,*fp1);
		energy = L_mac0(energy,lasts,lasts);

		cor2_exp = norm_l(cor);
		s = extract_h(L_shl(cor, cor2_exp));
		cor2_exp = shl(cor2_exp, 1);
		cor2 = extract_h(L_mult(s, s));
		ener_exp = norm_l(energy);
		ener = extract_h(L_shl(energy, ener_exp));

#if WMOPS
      test(); /* for the conditions in the IF below */
#endif
		IF ((cor > 0) && (ener>0)) {
			a0 = L_mult(cor2, energymax);
			a1 = L_mult(cor2max, ener);
			s = add(cor2_exp, energymax_exp);
			t = add(cor2max_exp, ener_exp);

         tt = sub(s, t);
			if (tt>=0) a0 = L_shr(a0, tt);
         if (tt<0)  a1 = L_shl(a1, tt); 

			IF (L_sub(a0, a1)>0) {
				cormax_local=L_max(cor,cor);
				cor2max = cor2; cor2max_exp = cor2_exp;
				energymax = ener; energymax_exp = ener_exp;
				energymax32_local = L_max(energy,energy);
				ppfe=k;
#if WMOPS
				move16();move16();move16();move16();move16();
#endif
			}
		}
	}

    /* USE THE RATIO OF AVERAGE MAGNITUDE AS THE SCALING FACTOR (PITCH TAP) */
	y = &xqp[sub(sub(xqoff, (*wsz)), ppfe)];/* candidate vector at delay ppfe */
	amy = L_mult(0,0);
	fp0 = y;
	FOR (k=0;k<(*wsz);k++) 
   {
      s = abs_s(*fp0++);
      amy = L_add(amy, (Word32)s);
	}

	IF (amy == 0)
   {
      t = 0;
#if WMOPS
      move16();
#endif
   }
	ELSE {
		fp0 = x;
		a1 = L_mult(0,0);
		FOR (k=0;k<(*wsz);k++) {
         s = abs_s(*fp0++);
         a1 = L_add(a1,(Word32)s);
		}

		/* t = (a1/amy); */
		ub = sub(norm_l(a1),1);
		lb = norm_l(amy);
		t = extract_h(L_shl(a1,ub));
		s = extract_h(L_shl(amy,lb));
		t = div_s(t, s);
		lb = sub(sub(lb,ub),1);		/* 15-14=1 */
		t = shl(t, lb);
		if (cormax_local < 0) t = negate(t);
	}

    /* LIMIT THE RANGE OF PITCH TAP to [-1, 1] */
	t = s_min(t,UPBOUND);
	t = s_max(t, DWNBOUND);
	*ptfe = t;
#if WMOPS
	move16();
#endif

	t = mult(768, *ptfe); /* Q10 * Q14 -> Q9 */
	t = s_max(t, 0);
	*ppt = t;
#if WMOPS
	move16();
#endif

#if (DMEM)
	/* memory deallocation */
	deallocWord16(xqbuf, 0, xqoff-1);
#endif

	*energymax32 = energymax32_local;
	*cormax = cormax_local;
#if WMOPS
	move32();move32();
#endif

	return ppfe;
}
