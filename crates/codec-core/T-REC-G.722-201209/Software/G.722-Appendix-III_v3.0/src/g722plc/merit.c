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
 * Function: merit()
 *
 * Description: This function calculates the figure of MERIT which determines 
 *              the mixing ratio of periodically extrapolated waveform and
 *              filtered white Gaussian noise.
 *
 * Inputs:  *xq - pointer to 16 kHz output speech buffer
 *          wsz - adaptive Window SiZe chosen for pitch refinement
 *          cormax - CORrelation MAXimum during pitch refinement search
 *          energymax32 - ENERGY MAXimum during pitch refinement search
 *          level - long-term average logarithmic signal level
 *          sflag - Shift FLAG for xq[]
 * Outputs: (return value of the function) - calculated figure of merit 
 *---------------------------------------------------------------------------*/
Word16  merit(
Word16  *xq,                /* (i) quantized signal from last sub-frame */
Word16  wsz,
Word32  cormax,
Word32  energymax32,
Word16  sflag)              /* (i) signaling for if energymax were scaled */
{
	Word32	a0, a1;
	Word16	*xqp;
	Word16  *fp0;
	Word16	s, t;
	Word16	cor2max, cor2max_exp;
	Word16	energymax, energymax_exp;
	Word32	sige, rese, r1;
	Word32	sigel;	/* Q25 */
	Word16	sigel_exp, sigel_frct, rho1;
	Word16	pg;	
	Word16	nlg;	   /* Q8 */
	Word16 	lb, ub;
	Word16	i, k;

#if (DMEM)
	Word16 *xqbuf;
#else
	Word16 xqbuf[XQOFF];
#endif

#if (DMEM)
	/* memory allocation */
	xqbuf = allocWord16(0, XQOFF-1);
#endif

	IF (sflag>0) 
   {
	    xqp = xqbuf;
	    FOR (i=0;i<XQOFF;i++) 
       {
          xqbuf[i] = shr(xq[i],sflag);
#ifdef WMOPS
          move16();
#endif
       }
	} 
   ELSE
   {
	    xqp = xq;
	}

   /* CALCULATE LOG-GAIN, PITCH PREDICTION GAIN, & FIRST NORMALIZED AUTOCORR */
	sige = 0;
	r1 = 0;
	fp0 = &xqp[XQOFF-wsz];
#if WMOPS
   move16();move16();
#endif
	FOR (k=0;k<wsz;k++) 
   {
		sige = L_mac0(sige,fp0[0],fp0[0]); /* prediction target SIGnal Energy */ 
		r1 = L_mac0(r1,fp0[0],fp0[-1]);    /* first autocorrelation coeff. */ 
		fp0++;
	}

	cor2max_exp = norm_l(cormax);
	s = extract_h(L_shl(cormax, cor2max_exp));
	cor2max_exp = shl(cor2max_exp, 1);
	cor2max = extract_h(L_mult(s, s));
	energymax_exp = norm_l(energymax32);
	energymax = extract_h(L_shl(energymax32, energymax_exp));

	IF (sige != 0) {
      /* calculate base-2 logarithm of signal energy */
		Log2(sige, &sigel_exp, &sigel_frct);  
		s = sigel_exp;
      move16();
		if (sub(sflag, 1)==0) 
         s = add(s, 2);
		sigel = L_shl(L_Comp(s, sigel_frct), 9);	/* Q25 */

		IF (energymax32 != 0) {

         /* calcualte pitch prediction residual energy "rese" */ 
			/* rese = sige-cormax*cormax/energymax; */
			/* t = cormax/energymax32; */
			a1 = L_abs(cormax);
			ub = sub(norm_l(a1),1);
			lb = norm_l(energymax32);
			t = extract_h(L_shl(a1,ub));
			s = extract_h(L_shl(energymax32,lb));
			t = div_s(t, s);
			lb = sub(sub(lb,ub),1);	/* Q14 */
			t = shl(t, lb);

			if (cormax < 0) t = negate(t);
			L_Extract(cormax, &ub, &lb);
			a0 = L_shl(Mpy_32_16(ub, lb, t),1);	/* cormax*cormax/energymax */
			rese = L_sub(sige, a0);

			IF (rese != 0) {

            /* calcualte pitch prediction gain */ 
				/* 10*log10(sige/rese) = 3.0103*(log2(sige)-log2(rese)) */
				Log2(rese, &s, &t);
				a0 = L_Comp(sigel_exp, sigel_frct);
				a1 = L_Comp(s, t);
				a0 = L_sub(a0, a1); /* Q16 */
				L_Extract(a0, &s, &t);
				a0 = Mpy_32_16(s, t, 24660);	/* 3.0103/4 Q15, a0 is Q14 */
				pg = round(L_shl(a0,11));		/* Q9 */	

			} ELSE pg = shl(20, 9);	/* Q9 */

		} ELSE
      {
         pg = 0;
#if WMOPS
         move16();
#endif
      }

		/* first normalized autocorrelation coefficient rho1 = r1/sige; */
		a1 = L_abs(r1);
		ub = sub(norm_l(a1),1);
		lb = norm_l(sige);
		t = extract_h(L_shl(a1,ub));
		s = extract_h(L_shl(sige,lb));
		t = div_s(t, s);
		lb = sub(lb,ub);	/* Q15 */
		rho1 = shl(t, lb);

		if (r1 < 0) rho1 = negate(rho1);
	} ELSE {
		sigel = 0;
		pg = 0;
		rho1 = 0;
#if WMOPS
      move16();move16();move16();
#endif
	}

	/* calculate the figure of merit: merit = nlg + pg + 12*rho1 */
	a0 = L_shr(L_sub(sigel, (Word32)0x1b000000),1);		/* Q24 */	
	nlg = round(a0);		/* nlg = normalized logarithmic gain in Q8 format */
	a0 = L_mac(a0, pg, 16384);		/* 1.0 Q14, pg Q9 -> Q24 */
	a0 = L_mac(a0, rho1, 3072);		/* 12. Q8, rho1 Q15 -> Q24 */

#if (DMEM)
	/* memory deallocation */
	deallocWord16(xqbuf, 0, XQOFF-1);
#endif

	return round(a0);		/* Q8 merit */
}
