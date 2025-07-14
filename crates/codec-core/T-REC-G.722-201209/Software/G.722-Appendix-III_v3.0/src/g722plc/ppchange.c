/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include <math.h>
#include "g722plc.h"
#include "typedef.h"
#include "utility.h"
#include "ppchange.h"
#include "memutil.h"
#include "table.h"

#define LBO 12  /* Low band Offset in 16kHz domain */

/* local function definitions */
void extractbuf(short *xq, short *esb, short D, short L, short pp);
Word16 getlag(Word16 *x, Word16 *esb, Word16 lsw, Word16 delta, Word16 *pemax,
              Word16 *pc2max);

/*-----------------------------------------------------------------------------
 * Function: testrpc()
 *
 * Description: Test to determine if the last good frame before erasure or the
 *              first good frame after erasure is completely unvoiced or 
 *              noise.  If either one is, return a flag=0 indicating that 
 *              rephasing and time warping should not be done.  Otherwise
 *              return a flag=1 indicating that rephasing and time warping 
 *              can be done.
 *
 * Inputs:  merit - figure of merit for the last good frame
 *          *inbuf- pointer to buffer containing the first good frame speech
 *                  of length FRSZ/2 (8kHz sampling).
 *
 * Outputs: return() - flag = 0 : last good frame or first good frame is 
 *                                unvoiced.
 *                   - flag = 1 : last good frame and first good frame are
 *                                not unvoiced.
 *---------------------------------------------------------------------------*/
int testrpc(short merit, short *inbuf)
{
   Word32   r0, r1;
   int      i;
   int      rpcflag;
   Word32   energy;
   Word16   shift, t;

	/* CHECK IF THE SIGNAL SEGMENT CAUSES OVERFLOW */
   t = shr(inbuf[0], 3);
   energy = L_mult0(t,t);
	FOR (i=1;i<FRSZ/2;i++) 
   {
	    t = shr(inbuf[i], 3);
	    energy = L_mac0(energy,t,t);
	}
   shift = norm_l(energy);
   shift = sub(6, shift);
   IF (shift > 0)
   {
      shift = shr(add(shift, 1), 1);
      FOR (i=0;i<FRSZ/2;i++)
      {
         inbuf[i] = shr(inbuf[i], shift);
#ifdef WMOPS
         move16();
#endif
      }
   }
   shift = s_max(shift, 0);

   /* IF THE LAST GOOD FRAME IS UNVOICED, DO NOT DO WAVEFORM MATCHING */
   IF (sub(merit, 256*MLO)<=0)
   {
      rpcflag=0;
#if WMOPS
      move16();
#endif
   }
   ELSE
   {
      /* % IF FIRST FUTURE GOOD FRAME LOOKS LIKE UNVOICED, SKIP WAVEFORM MATCHING */
      r0 = L_mult0(inbuf[FRSZ/2-1], inbuf[FRSZ/2-1]); 
      r1 = L_mult(0,0);
      FOR (i=0; i<FRSZ/2-1; i++)
      {
         r0 = L_mac0(r0, inbuf[i], inbuf[i]);     /* % r0 = energy of 1st future good frame  */
         r1 = L_mac0(r1, inbuf[i], inbuf[i+1]);   /* % r1 = 1st unnormalized autocorrelation */
      }     
      r0 = L_shr(r0, 3); 
      r0 = L_sub(r1,r0);
      if (r0<0)               /* % if 1st normalized autocorrelation coefficient < 0.1,  */
         rpcflag=0;           /* % don't do future waveform matching; signal it by wmflag=0 */
      if (r0>=0)              /* % otherwise, */
         rpcflag=1;           /* % do future waveform matching; signal it by setting wmflag=0 */

#if WMOPS
      move16();
#endif
   }
   return(rpcflag);
} 

/*-----------------------------------------------------------------------------
 * Function: resample()
 *
 * Description: Low complexity resampler.  The input buffer is stretched or
 *              shrunk by "delta" samples.  The resampling is done by a 
 *              sample shift overlap-add process.  The resulting signal is 
 *              placed in the output buffer.  In the case of stretching
 *              (delta > 0), any extra samples beyond FRSZ are not computed.
 *              The extra samples to the "left" are the ones thrown out.
 *
 * Inputs:  *in   - pointer to 16kHz input buffer
 *          *out  - pointer to buffer for output
 *          delta - number of samples to stretch (+) or shrink (-)
 *
 * Outputs: *out  - "resampled" signal.
 *---------------------------------------------------------------------------*/
void resample(Word16 *in, Word16 *out, Word16 delta)
{
   Word16   olalen;
   Word16   i;
   Word16   iola;
   Word16   ad;      /* Add/Drop */
   Word16   inlen;
   Word16   skip;
   Word16   outi;
   Word16   oldi;

   Word16 spad16;
   Word32 nspad32;
   Word16 Qspad;
   Word16 Qinlen;
   Word16 Qdelta;
   Word16 temp, temp2;
   Word16 tempQ;
   Word16 *pup;
   Word16 *pdwn;
   Word16 olandx;
   Word32 a0;
   Word16 *ola3_8[6] = {ola3, ola4, ola5, ola6, ola7, ola8};

   /* Compute the number of samples in the input that will be used */
   /* Also, compute the samples per add/drop needed, and the ola length */
   IF (delta !=0)
   {
      ad = 1;  /* add */
      inlen = (FRSZ - MIN_UNSTBL);
      skip = 0;
#if WMOPS
      move16();move16();move16();
#endif
      IF (delta < 0) /* indicates a drop */
      {
         delta = sub(0, delta);
         ad = -1; /* drop */
#if WMOPS
      move16();
#endif
      }
      IF (sub(ad,1)==0)
      { 
         IF (sub(delta,MIN_UNSTBL)>0)
         {
            skip  = sub(delta, MIN_UNSTBL);
            inlen = sub(inlen, skip);
         }
      }

      /* spad = ((Float)inlen)/delta; */
      Qdelta = norm_s(delta);
      Qinlen = norm_s(inlen);
      temp2  = shl(delta, Qdelta);
      temp   = shl(inlen, Qinlen);
      IF (sub(temp,temp2)>=0)
      {
         temp  = shr(temp, 1);
         Qinlen = sub(Qinlen, 1);
      }
      spad16 = div_s(temp, temp2);
      Qspad  = sub(Qinlen, Qdelta);
      Qspad  = add(Qspad, 15);

      IF (add(ad,1)==0)    /* compute olalen for stretching */
      {
         /* olalen = (int)((inlen-delta)/delta);*/  /* floor */
         temp = sub(inlen, delta);
         tempQ= norm_s(temp);
         temp = shl(temp, tempQ);
         IF (sub(temp,temp2)>=0)
         {
            temp = shr(temp, 1);
            tempQ = sub(tempQ, 1);
         }
         olalen = div_s(temp, temp2);
         temp   = sub(tempQ, Qdelta);
         temp   = add(temp, 15);
         olalen = shr(olalen, temp);
      }
      ELSE                 /* compute olalen for shrinking */
      {
         temp = shr(spad16, Qspad);
         temp2 = sub(spad16, shl(temp, Qspad));
         if (temp2!=0)
            olalen = add(1, temp);
         if (temp2==0)
         {
            olalen = temp;
#if WMOPS
            move16();
#endif
         }
      }
      if (sub(olalen,8)>0) /* limit the olalen to 8 */
      {
         olalen=8;
#if WMOPS
         move16();
#endif
      }

      outi = 0;

      spad16  = add(spad16, 1);
      nspad32 = L_shl(skip, Qspad);
      olandx  = sub(olalen, 3);

#if WMOPS
      move16();
#endif
      /* Do the actual sample shift OLA resampling */
      FOR(i=skip; i<FRSZ-MIN_UNSTBL;i++) /* loop through the input */
      {
#if WMOPS
         test();    /* for the 2nd test in the IF */ 
#endif
         /* time to add/delete a sample ? */
         IF ((L_sub(i, L_shr(nspad32, Qspad))==0)&&(sub(i,FRSZ-MIN_UNSTBL-1))!=0)    
         {
            nspad32 = L_add(nspad32, spad16); /* increment for next time */
            iola = 0;
            oldi = i;
            i = sub(i, ad);
            pup = ola3_8[olandx];
            pdwn = &pup[olalen-1];
#if WMOPS
            move16();move16();
#endif
         }
         IF (sub(iola,olalen)<0)  /* we are in the middle of an ola */
         {
            iola = add(iola, 1);
 			   a0 = L_mult(in[i],*pup++);
			   a0 = L_mac(a0, in[oldi++], *pdwn--);
		  	   out[outi++] = round(a0); 
#ifdef WMOPS
            move16();      
#endif
         }
         ELSE     /* not in an ola, so just copy the sample */
         {
            out[outi++] = in[i];
#if WMOPS
            move16();
#endif
         }
      }
   }
   ELSE     /* delta=0, so just copy the samples => no warping */
   {
      W16copy(out, in, FRSZ-MIN_UNSTBL);
   }
}

/*-----------------------------------------------------------------------------
 * Function: extractbuf()
 *
 * Description: Extract an extrapolated signal from the output history buffer.
 *
 * Inputs:  *xq   - pointer to the history buffer
 *          *esb  - pointer to buffer for output
 *          D     - Distance from end of last frame to start of esb()
 *          L     - number of samples to generate from xq
 *          pp    - pitch period to be used for PWE.
 *
 * Outputs: *esb  - extrapolated signal.
 *---------------------------------------------------------------------------*/
void extractbuf(Word16 *xq, Word16 *esb, Word16 D, Word16 L, Word16 pp)
{
   Word16 pos;
   int    n;
   Word16 ovs;

#if WMOPS
   move16();
#endif
   pos=0;      /* initialize position to first sample of current frame */
   IF (D<0)    /* start with known samples back in xq */
   {
      W16copy(esb, xq+XQOFF+D, -D); /* copy the old samples into esb */
      IF (sub(add(L,D),pp) <= 0)   /* if # of remaining samples in esb() <= pitch period */
      {
         W16copy(esb-D, xq+XQOFF-pp, L+D);      /* fill rest of esb() in 1 shot */
      }
      ELSE
      {
         W16copy(esb-D, xq+XQOFF-pp, pp);     /* copy one pitch cycle first, */
         n= sub(pp, D);

         FOR (;n<L;n++)
         {
            esb[n] = esb[n-pp];           /*  then fill the rest of esb() buffer */
#if WMOPS
            move16();
#endif
         }
      }
   }
   ELSE        /* starting point is some point in the future */
   {
      WHILE (sub(pos,D) < 0)
      {
         pos=add(pos, pp);    /* increment pos by one pitch period at a time  */
      }                 /*  until position fly pass D */
      ovs=sub(pos, D);        /*  overshoot = last position - D */
      IF (sub(ovs,L) >= 0)     /* if pos landed on esb(L) or beyond, */
      {
         W16copy(esb, xq+XQOFF-ovs, L); /* copy entire esb() in one shot*/
      }
      ELSE              /* if pos landed inside the esb() buffer */
      {
         IF (ovs > 0)   /* if pos landed beyong esb(1),*/
         {
            W16copy(esb, xq+XQOFF-ovs, ovs);    /* copy samples before landing position */
         }
         IF (sub(L,ovs) <= pp)   /* if # of remaining samples in esb() <= pitch period */
         {
            W16copy(esb+ovs, xq+XQOFF-pp, L-ovs);      /* fill rest of esb() in 1 shot */
         }
         ELSE
         {
            W16copy(esb+ovs, xq+XQOFF-pp, pp);     /* copy one pitch cycle first, */
            n=add(ovs, pp);

            FOR (;n<L;n++)
            {
               esb[n] = esb[n-pp];           /*  then fill the rest of esb() buffer */
#if WMOPS
               move16();
#endif
            }
         }
      }
   }
}

/*-----------------------------------------------------------------------------
 * Function: refinelag()
 *
 * Description: Refine the estimated lag using only the data within the OLA 
 *              window.  Estimate the position of the OLA window by the current
 *              value of the lag.
 *
 * Inputs:  *xq   - pointer to the history buffer
 *          pp    - pitch period to be used for PWE.
 *          *inbuf- pointer to the first good frame 16kHz data
 *          estlag- current lag estimate
 *
 * Outputs: (int) - the refined lag.
 *---------------------------------------------------------------------------*/
#define RSR    4    /* refine search range 16kHz */
int refinelag( short *xq, short pp, short *inbuf, short estlag)
{
#if DMEM
   Word16 *esb;
   Word16 *y;
#else
   Word16 esb[OLALG+2*RSR];
   Word16 y[OLALG];			/* scaled inbuf[MIN_UNSTBL...MIN_UNSTBL+OLALG-1] */
#endif

   Word16 L, D, lagmax;
   Word16 e[2], c2[2];

   Word16 *p_y, *p_in, *p_esb;
   Word16 i, t, shift_y, shift_esb;
   Word32 e_esb, e_y;
   Word16 spola; /* start position of ola */

#if DMEM
   esb = allocWord16(0, OLALG+2*RSR-1);
   y = allocWord16(0, OLALG-1);
#endif
   /* EXTRAPOLATE WAVEFORM TO FUTURE GOOD FRAME(S) +- del SAMPLES */
   L=add(OLALG,RSR<<1);      /* Length of extrapolated signal buffer for waveform matching */

   spola = add(estlag, (FRSZ-MIN_UNSTBL));
   spola = s_min(spola,FRSZ);
   spola = sub(FRSZ, spola);

   D=sub(sub(spola,estlag),RSR);      /* Distance from end of last frame to start of esb() */
   extractbuf(xq, esb, D, L, pp);

   /* scale input */
   p_in = inbuf+spola;
   t = shr(*p_in++, 3);
   e_y = L_mult0(t,t);
   FOR(i=1;i<OLALG;i++){
	   t = shr(*p_in++, 3);
	   e_y = L_mac0(e_y, t, t);
   }
   shift_y = norm_l(e_y);
   shift_y = sub(6, shift_y);
   t = add(shift_y, 2);
   if(shift_y >= 0)
	   shift_y = shr(t, 1);
   if (shift_y<0)
   {
	   shift_y = 0;
#if WMOPS
	   move16();
#endif
   }
   p_in = inbuf+spola;
   p_y  = y;

   FOR(i=0;i<OLALG;i++)
   {
	   *p_y++ = shr(*p_in++, shift_y);
#ifdef WMOPS
      move16();
#endif
   }

   /* scale esb */
   p_esb = esb;
   t = shr(*p_esb++, 3);
   e_esb = L_mult0(t,t);
   FOR(i=1;i<OLALG+2*RSR;i++){
	   t = shr(*p_esb++, 3);
	   e_esb = L_mac0(e_esb, t, t);
   }
   shift_esb = norm_l(e_esb);
   shift_esb = sub(6, shift_esb);
   t = add(shift_esb, 2);
   if(shift_esb >= 0)
	   shift_esb = shr(t, 1);
   if (shift_esb<0)
   {
	   shift_esb = 0;
#if WMOPS
	   move16();
#endif
   }
   p_esb = esb;

   FOR(i=0;i<OLALG+2*RSR;i++){
	   *p_esb = shr(*p_esb, shift_esb);
#ifdef WMOPS
      move16();
#endif
	   p_esb++;
   }

   /* refine the lag */
   lagmax = getlag(y, esb, OLALG, RSR, e, c2);

#if DMEM
   deallocWord16(esb, 0, OLALG+2*RSR-1);
   deallocWord16(y, 0, OLALG-1);
#endif

   /* new lag is the old lag added to the new lag */
   return (add(lagmax, estlag));
}
/*-----------------------------------------------------------------------------
 * Function: ppchange()
 *
 * Description: Compute the lag offset between an extrapolated signal based on
 *              the output history buffer, and the first good frame.
 *
 * Inputs:  *xq   - pointer to the history buffer
 *          pp    - pitch period to be used for PWE.
 *          *inbuf- pointer to the first good frame 16kHz data
 *          estlag- current lag estimate
 *
 * Outputs: (int) - the refined lag.
 *---------------------------------------------------------------------------*/
#define REF 4 /* 16 kHz */
int ppchange( short *xq, short pp, short *inbuf)
{
#if DMEM
   Word16 *esb;
   Word16 *esb4k;
   Word16 *in4k;
#else
   Word16    esb[(FRSZ+2*MAXOS)];     /* extrapolated signal buffer */
   Word16    esb4k[(FRSZ+2*MAXOS)/4];
   Word16    in4k[FRSZ/4];
#endif
   Word16    LSW, L, D;
   Word16      i, n;


   Word16      lagmax, refinement;
   Word16      del;
   Word16 emax_fx[2], cor2max_fx[2];
   Word32   ee;
   Word32 energy;
   Word16   t,tt;
   Word16   shift;
   Word16 ee_exp, ee_man;
   Word16 *ps;

#if DMEM
   esb = allocWord16(0, (FRSZ+2*MAXOS)-1);
   esb4k = allocWord16(0, ((FRSZ+2*MAXOS)/4)-1);
   in4k = allocWord16(0, (FRSZ/4)-1);
#endif
   del = shr(add(pp, 1), 1);
   del = add(del, 3);

   t = shl(shr(del,2),2);
   if ( sub(t,del)!=0) 
   {
      del=t;
#if WMOPS
      move16();
#endif
   }
   del = s_min(del, MAXOS);

   /* Set the Lag Search Window */
   LSW = add(pp, shl(pp, 1));
   LSW = shr(add(LSW, 1), 1);
   LSW = s_max(LSW, 80);
   LSW = s_min(LSW, FRSZ);
   LSW = shr(LSW, 1);  /* now at 8k */

   /* EXTRAPOLATE WAVEFORM TO FUTURE GOOD FRAME(S) +- del SAMPLES */
   /* Length of extrapolated signal buffer for waveform matching */
   L = shl(add(LSW, del), 1);

   /* Distance from end of last frame to start of esb() */
   D = sub(LBO, del);

   extractbuf(xq, esb, D, L, pp);

	/* CHECK IF THE SIGNAL SEGMENT CAUSES OVERFLOW */
   t = shr(esb[0], 3);
   energy = L_mult0(t,t);
   FOR (i=2;i<L>>1;i+=2) 
   {
	    t = shr(esb[i], 3);
	    energy = L_mac0(energy,t,t);
	}
   shift = norm_l(energy);
   shift = sub(6, shift);
   t = add(shift, 2);
   if (shift >= 0)
   {
      shift = shr(t, 1);
   } 
   if (shift<0)
   {
      shift = 0;
#if WMOPS
      move16();
#endif
   }

   /* subsample esb to 4k */
   t = sub(shift, 1);
   if (t<0) 
   {
      t=0;
#if WMOPS
      move16();
#endif
   }
   ps = esb;
   FOR (i=0;i<(L>>2);i++)
   {
      esb4k[i] = shr(*ps, t);
#ifdef WMOPS
      move16();
#endif
      ps+=4;
   }

   /* subsample input to 4k */
   ps = inbuf;
   FOR (i=0;i<(FRSZ>>2);i++)
   {
      in4k[i] = *ps;
      ps+=2;
#if WMOPS
      move16();
#endif
   }

   lagmax = getlag(in4k, esb4k, shr(LSW,1), shr(del, 2), emax_fx, cor2max_fx);

   lagmax = shl(lagmax, 2);  /* 16kHz domain */

   /* refine the lag to 8kHz */
   lagmax = s_min(lagmax, add(del, -REF));
   lagmax = s_max(lagmax, sub(REF, del));

   /* first, subsample esb to 8kHz but offset by our lag */
   L = shl(add(LSW, REF), 1);
   n = sub(sub(del, lagmax),REF);
   ps = &esb[n];
   FOR (i=0;i<shr(L, 1);i++)
   {
      esb[i] = shr(*ps, shift);
#ifdef WMOPS
      move16();
#endif
      ps+=2;
   }
   refinement = getlag(inbuf, esb, LSW, REF>>1, emax_fx, cor2max_fx);
   lagmax = add(lagmax, shl(refinement, 1));

   ee = L_mult0(inbuf[0], inbuf[0]);
   FOR (i=1;i<LSW;i++)
      ee = L_mac0(ee, inbuf[i], inbuf[i]);

	ee_exp = norm_l(ee);
	t = extract_h(L_shl(ee, ee_exp));

	ee_exp = add(ee_exp, emax_fx[1]);
	ee_man = extract_h(L_mult(t, emax_fx[0]));
   
   tt = sub(ee_exp, cor2max_fx[1]);
   if (tt>0)
      ee_man = shr(ee_man, tt);
   if (tt<0)
   {
      cor2max_fx[0] = shl(cor2max_fx[0], tt);
#ifdef WMOPS
      move16();
#endif
   }

   IF (ee_man == 0)              /* % in degenerate case of zero vectors */
   {
#if WMOPS
      move16();
#endif
      lagmax=-100;               /* %   reset best lag to 0 */
   }
   ELSE IF (( sub(lagmax,(MAXOS-2))>0 )||( sub(lagmax,(-MAXOS+2))<0 )) /* don't trust near the boundary */
   {
#if WMOPS
      move16();test();
#endif
      lagmax=-100;
   }
   ELSE
   {

      if (sub(cor2max_fx[0], shr(ee_man, 2))<=0) /* % 0.09 else if cos(theta) of two vectors < 0.3, */
      {
#if WMOPS
         move16();
#endif
         lagmax=-100;               /* %   reset best lag to 0 */
      }
   }
#if DMEM
   deallocWord16(esb, 0, (FRSZ+2*MAXOS)-1);
   deallocWord16(esb4k, 0, ((FRSZ+2*MAXOS)/4)-1);
   deallocWord16(in4k, 0, (FRSZ/4)-1);
#endif
   return(lagmax);
}

/*-----------------------------------------------------------------------------
 * Function: getlag()
 *
 * Description: Find the lag that maximizes the cross correlation between the
 *              signal in x and the signal in esb.  Use a window of length
 *              lsw and search +-delta samples.  The function getlag assumes 
 *              input vectors are properly shifted outside for optimal 
 *              saturation/precision tradeoff 
 *
 * Inputs:  *x    - pointer to the fixed signal buffer
 *          *esb  - pointer to extrapolated signal buffer.  It contains enough
 *                  signal to search +-delta samples.
 *          lsw   - lag search window length
 *          delta - search for the maximum lag within a range of +-delta samps
 *
 * Outputs: (Word16) - the lag.
 *          *pemax - value contains the energy of esb at the returned lag.
 *          *pc2max- value contains the correlation^2 at the returned lag.
 *---------------------------------------------------------------------------*/
Word16 getlag(Word16 *x, Word16 *esb, Word16 lsw, Word16 delta, Word16 *pemax, Word16 *pc2max)
{
	Word16 *p_y, *p_y_lsw, *p_y1, *p_x;
	Word16 c2max_man, c2max_exp;
	Word16 emax_man, emax_exp;
	Word16 e_man, e_exp;
	Word16 c2_man, c2_exp;
	Word32 e, c, a0, a1;
	Word16 i, lag, lagmax, s, t, tt;

	/* EXTRACT FIRST GOOD FRAME(S)  */
	/* AND FIND ITS TIME LAG RELATIVE TO EXTRAPOLATED WAVEFORM */
  
	p_y = esb;
	p_x = x;

	e = L_mult0(*p_y, *p_y);	   
	c = L_mult0(*p_x++, *p_y++);
	FOR(i=0; i<lsw-1; i++){
		e = L_mac0(e, *p_y, *p_y);       /*  energy of y()*/
		c = L_mac0(c, *p_x++, *p_y++);   /*  correlation at first lag of MAXOS */
	}

	c2max_exp = norm_l(c);
	s = extract_h(L_shl(c, c2max_exp));
	c2max_exp = shl(c2max_exp, 1);
	c2max_man = extract_h(L_mult(s, s));
	if(c < 0)                           /* if correlation is negative, */
		c2max_man = sub(0, c2max_man);   /* get correlation square with negative sign */

	emax_exp = norm_l(e);
	emax_man = extract_h(L_shl(e, emax_exp));

	lagmax = delta; 
	p_y = esb;
	p_y_lsw = p_y+lsw;
#if WMOPS
	move16();
#endif
	FOR(lag=delta-1; lag>=-delta; lag--){
		e = L_msu0(e, *p_y, *p_y);
		e = L_mac0(e, *p_y_lsw, *p_y_lsw);
		p_y++; p_y_lsw++;

		p_y1 = p_y;
		p_x  = x;

		c = L_mult0(*p_x++, *p_y1++);
		FOR(i=0;i<lsw-1;i++){
			c = L_mac0(c, *p_x++, *p_y1++);	/*  correlation at lag */
		}
	
		c2_exp = norm_l(c);
		s = extract_h(L_shl(c, c2_exp));
		c2_exp = shl(c2_exp, 1);
		c2_man = extract_h(L_mult(s, s));
		if(c < 0)							/* if correlation is negative, */
			c2_man = sub(0, c2_man);		/* get correlation square with negative sign */
		e_exp = norm_l(e);
		e_man = extract_h(L_shl(e, e_exp));

		a0 = L_mult(c2_man, emax_man);
		a1 = L_mult(c2max_man, e_man);
		s = add(c2_exp, emax_exp);
		t = add(c2max_exp, e_exp);

      tt = sub(s, t);
      if (tt >=0)
         a0 = L_shr(a0, tt);
      if (tt <0)
         a1 = L_shl(a1, tt); 

		IF (L_sub(a0, a1) > 0) {
			c2max_man = c2_man; c2max_exp = c2_exp;	
			emax_man = e_man; emax_exp = e_exp;
			lagmax = lag;
#if WMOPS
            move16();move16();;move16();move16();;move16();
#endif
		}

	}
   
	pemax[0]  = emax_man; pemax[1]   = emax_exp;
	pc2max[0] = c2max_man; pc2max[1] = c2max_exp;
#if WMOPS
	move16();;move16();move16();;move16();
#endif
   
	return(lagmax);
}

