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
 * Function: coarsepitch()
 *
 * Description: COARSE PITCH period extraction.
 *              This function extracts the coarse pitch period based on the 
 *              2 kHz decimated weighted speech signal.
 *
 * Inputs:  *xwd  - pointer to 2 kHz decimated weighted speech buffer
 *          cpplast - coarse pitch period of the last frame
 * Outputs: (return value of the function) - extracted coarse pitch period
 *---------------------------------------------------------------------------*/
Word16   coarsepitch(
Word16 	*xwd,				
Word16	cpplast)
{
  Word16    s;       /* Q3 */
  Word16    a, b;
  Word16    im;
  Word16    maxdev, flag, mpflag;
  Word32    eni, deltae;
  Word32    cc;
  Word16    ah,al, bh, bl;
  Word32    *cor, *energy;
  Word16    *cor2, *cor2_exp;
  Word32    a0, a1, a2, a3;
  Word16    *fp0, *fp1, *fp2, *fp3;
  Word16    cor2max, cor2max_exp;
  Word16    cor2m, cor2m_exp;
  Word16    s0, t0, t1, exp0, exp1, e2, e3,tt;
  Word16    threshold;
  Word16	   mplth;		/* Q3 */
  
  Word16    i, j, k, n, npeaks, imax, idx[MAXPPD-MINPPD+1];
  Word16    cpp;
  
#if (DMEM)
  Word16 *plag, *_cor2, *_cor2_exp;
  Word16 *cor2i, *cor2i_exp;
  Word32 *_cor, *_energy;
#else
  Word16 plag[HMAXPPD], _cor2[MAXPPD1+1], _cor2_exp[MAXPPD1+1];
  Word16 cor2i[HMAXPPD], cor2i_exp[HMAXPPD];
  Word32 _cor[MAXPPD1+1], _energy[MAXPPD1+1];
#endif
 
#if (DMEM)
  Word16 *_energy_man, *_energy_exp;
  Word16 *energyi_man, *energyi_exp;
#else
  Word16 _energy_man[MAXPPD1+1], _energy_exp[MAXPPD1+1];
  Word16 energyi_man[HMAXPPD], energyi_exp[HMAXPPD];
#endif
  Word16 *energy_man, *energy_exp;
  Word16 energym_man, energym_exp;
  Word16 energymax_man, energymax_exp;

  /* COMPUTE CORRELATION & ENERGY OF PREDICTION BASIS VECTOR */
#if (DMEM)
  /* memory allocation */
  _cor      = allocWord32(0, MAXPPD1);
  _energy   = allocWord32(0, MAXPPD1);
  _energy_man = allocWord16(0, MAXPPD1);
  _energy_exp = allocWord16(0, MAXPPD1);
  _cor2     = allocWord16(0, MAXPPD1);
  _cor2_exp = allocWord16(0, MAXPPD1);
#endif  

  cor = _cor+1;
  energy = _energy+1;
  cor2 = _cor2+1;
  cor2_exp = _cor2_exp+1;
  
  energy_man = _energy_man+1;
  energy_exp = _energy_exp+1;

  fp0 = &xwd[MAXPPD1];
  fp1 = &xwd[MAXPPD1-M1];
  a0 = L_mult0(*fp1, *fp1);
  a1 = L_mult0(*fp0++, *fp1++);
  FOR (i=1;i<PWSZD;i++) 
  {
    a0 = L_mac0(a0, *fp1, *fp1);
    a1 = L_mac0(a1, *fp0++, *fp1++);
  }
  
  cor[M1-1] = a1;
  energy[M1-1] = a0;
  energy_exp[M1-1] = norm_l(energy[M1-1]);
  energy_man[M1-1] = extract_h(L_shl(energy[M1-1], energy_exp[M1-1]));
  s0 = cor2_exp[M1-1] = norm_l(a1);
  t0 = extract_h(L_shl(a1, s0));
  tt = extract_h(L_mult(t0, t0));
#if WMOPS
   move32();move32();move32();move32();move16();
#endif

  if (a1 < 0) 
  {
     tt = negate(tt);
  }
  cor2[M1-1] = tt;
#ifdef WMOPS
  move16();
#endif
     
  fp2 = &xwd[LXD-M1-1];
  fp3 = &xwd[MAXPPD1-M1-1];

  FOR (i=M1;i<M2;i++) {
    fp0 = &xwd[MAXPPD1];
    fp1 = &xwd[MAXPPD1-1-i];
    a1 = L_mult0(*fp0++,*fp1++);
    FOR (j=1;j<(LXD-MAXPPD1);j++) a1 = L_mac0(a1,*fp0++,*fp1++); 
    cor[i] = a1;
    a0 = L_msu0(a0, *fp2, *fp2);
    a0 = L_mac0(a0, *fp3, *fp3);
    fp2--; fp3--;  /* pointer manipulation here can be avoided in a real implementation */
    energy[i] = a0;
    energy_exp[i] = norm_l(energy[i]);
    energy_man[i] = extract_h(L_shl(energy[i], energy_exp[i]));
    s0 = cor2_exp[i] = norm_l(a1);
    t0 = extract_h(L_shl(a1, s0));
    tt = extract_h(L_mult(t0, t0));
#if WMOPS
    move32();move32();move16();move16();move16();
#endif 
    if (a1 < 0) 
    {
       tt = negate(tt);
    }
    cor2[i] = tt;
#ifdef WMOPS
    move16();
#endif
  }
  
  /* FIND POSITIVE CORRELATION PEAKS */
  /* FIND MAXIMUM OF COR*COR/ENERGY AMONG POSITIVE CORRELATION PEAKS */ 
  npeaks = 0;
  n=MINPPD-1;
#if WMOPS
  move16();move16();
#endif 
  WHILE ((sub(n,MAXPPD)<0)&&(sub(npeaks,MAX_NPEAKS)<0)) 
  {
#ifdef WMOPS
    test();
#endif
    IF (cor[n]>0) 
    {
      a0   = L_mult(energy_man[n-1],cor2[n]);
      a1   = L_mult(energy_man[n], cor2[n-1]);
      exp0 = shl(sub(cor2_exp[n], cor2_exp[n-1]),1);
      exp0 = add(exp0, energy_exp[n-1]);
      exp0 = sub(exp0, energy_exp[n]);

      if (exp0>=0) 
	      a0 = L_shr(a0, exp0);
      if (exp0<0) 
	      a1 = L_shl(a1, exp0);

      IF (L_sub(a0, a1)>0) 
      { 
	      a0   = L_mult(energy_man[n+1],cor2[n]);
	      a1   = L_mult(energy_man[n], cor2[n+1]);
	      exp0 = shl(sub(cor2_exp[n], cor2_exp[n+1]),1);
	      exp0 = add(exp0, energy_exp[n+1]);
	      exp0 = sub(exp0, energy_exp[n]);

	      if (exp0>=0) 
	        a0 = L_shr(a0, exp0);
	      if (exp0<0) 
	        a1 = L_shl(a1, exp0);

	      IF (L_sub(a0, a1)>0) 
         {
	        idx[npeaks] = n;
	        npeaks=add(npeaks,1); 
#if WMOPS
           move16();
#endif
	      }
      }
    }
	n=add(n, 1);
  }

  /* if there are no positive peaks, repeat allowing negative peaks */
  IF (npeaks == 0){   
	  n=MINPPD-1;
#if WMOPS
	  move16();
#endif 

	  FOR (i=0;i<MAXPPD1;i++) {
		  cor2[i] = sub(0,cor2[i]);
		  cor[i]  = L_sub(0,cor[i]);
#ifdef WMOPS
        move16();move32();
#endif
	  }
  
	  WHILE ((sub(n,MAXPPD)<0)&&(sub(npeaks,MAX_NPEAKS)<0)) 
	  {
#ifdef WMOPS
        test();
#endif
		  IF (cor[n]>0) 
		  {
			  a0   = L_mult(energy_man[n-1],cor2[n]);
			  a1   = L_mult(energy_man[n], cor2[n-1]);
			  exp0 = shl(sub(cor2_exp[n], cor2_exp[n-1]),1);
			  exp0 = add(exp0, energy_exp[n-1]);
			  exp0 = sub(exp0, energy_exp[n]);
			  if (exp0>=0) 
				  a0 = L_shr(a0, exp0);
			  if (exp0<0) 
				  a1 = L_shl(a1, exp0);
			  IF (L_sub(a0, a1)>0) 
			  { 
				  a0   = L_mult(energy_man[n+1],cor2[n]);
				  a1   = L_mult(energy_man[n], cor2[n+1]);
				  exp0 = shl(sub(cor2_exp[n], cor2_exp[n+1]),1);
				  exp0 = add(exp0, energy_exp[n+1]);
				  exp0 = sub(exp0, energy_exp[n]);
				  if (exp0>=0) 
					  a0 = L_shr(a0, exp0);
				  if (exp0<0) 
					  a1 = L_shl(a1, exp0);
				  IF (L_sub(a0, a1)>0) 
				  {
					  idx[npeaks] = n;
					  npeaks=add(npeaks, 1); 
#if WMOPS
					  move16();
#endif
				  }
			  }
		  }
		  n=add(n,1);
	  }

	  IF (npeaks == 0){   /* if there are no positive AND no negative peaks, */
#if (DMEM)
		  /* memory deallocation */
		  deallocWord32(_cor, 0, MAXPPD1);
		  deallocWord32(_energy, 0, MAXPPD1);
		  deallocWord16(_energy_man, 0, MAXPPD1);
		  deallocWord16(_energy_exp, 0, MAXPPD1);
		  deallocWord16(_cor2, 0, MAXPPD1);
		  deallocWord16(_cor2_exp, 0, MAXPPD1);
#endif
		  return (i_mult(MINPPD, cpp_scale)); /* return minimum pitch period */
	  }
  }

  IF (sub(npeaks, 1)==0){   /* if there is exactly one peak, */
#if (DMEM)
    /* memory deallocation */
    deallocWord32(_cor, 0, MAXPPD1);
    deallocWord32(_energy, 0, MAXPPD1);
    deallocWord16(_energy_man, 0, MAXPPD1);
    deallocWord16(_energy_exp, 0, MAXPPD1);
    deallocWord16(_cor2, 0, MAXPPD1);
    deallocWord16(_cor2_exp, 0, MAXPPD1);
#endif

    /* return the time lag for this single peak */
    return (i_mult(add(idx[0],1), cpp_scale)); 
  }
  
#if (DMEM)
  /* memory allocation */
  plag      = allocWord16(0, HMAXPPD-1);

  energyi_man = allocWord16(0, HMAXPPD-1);
  energyi_exp = allocWord16(0, HMAXPPD-1);
  cor2i     = allocWord16(0, HMAXPPD-1);
  cor2i_exp = allocWord16(0, HMAXPPD-1);
#endif
  
  /* IF PROGRAM PROCEEDS TO HERE, THERE ARE 2 OR MORE PEAKS */
  cor2max=(Word16) 0x8000;
  cor2max_exp= (Word16) 0;
  energymax_man=1;
  energymax_exp=0;
  imax=0;
#if WMOPS
  move16();move16();move16();move16();move16();
#endif
  FOR (i=0; i < npeaks; i++) {
    
    /* FIND INTERPOLATED PEAKS OF cor2[]/energy[] USING QUADRATIC 
       INTERPOLATION FOR cor[] AND LINEAR INTERPOLATION FOR energy[]. */
    /* first calculate coefficients of quadratic function y(x)=ax^2+bx+c; */
    n=idx[i];
    a0=L_sub(L_shr(L_add(cor[n+1],cor[n-1]),1),cor[n]);
    L_Extract(a0, &ah, &al);
    a0=L_shr(L_sub(cor[n+1],cor[n-1]),1);
    L_Extract(a0, &bh, &bl);
    cc=L_max(cor[n],cor[n]);
    
    /* INITIALIZE VARIABLES BEFORE SEARCHING FOR INTERPOLATED PEAK */
    im=0;
    cor2m_exp = cor2_exp[n];
    cor2m = cor2[n];
    energym_exp = energy_exp[n];
    energym_man = energy_man[n];
    eni=L_max(energy[n],energy[n]);
#if WMOPS
    move16();move16();move16();move16();move16();move16();
#endif
  
    /* DERTERMINE WHICH SIDE THE INTERPOLATED PEAK FALLS IN, THEN
       DO THE SEARCH IN THE APPROPRIATE RANGE */
    
    a0	 = L_mult(energy_man[n-1],cor2[n+1]);
    a1 	 = L_mult(energy_man[n+1], cor2[n-1]);
    exp0 = shl(sub(cor2_exp[n+1], cor2_exp[n-1]),1);
    exp0 = add(exp0, energy_exp[n-1]);
    exp0 = sub(exp0, energy_exp[n+1]);
    if (exp0>=0) 
      a0 = L_shr(a0, exp0);
    if (exp0<0) 
      a1 = L_shl(a1, exp0);
    
    IF (L_sub(a0, a1)>0) 
    {	/* if right side */  
      deltae = L_shr(L_sub(energy[n+1], eni), 3);
      FOR (k = 0; k < HDECF; k++) 
      {
         a0=L_add(L_add(Mpy_32_16(ah,al,x2[k]),Mpy_32_16(bh,bl,x[k])),cc);
         eni = L_add(eni, deltae);
         a1 = L_max(eni, eni);
         exp0 = norm_l(a0);
         s0 = extract_h(L_shl(a0, exp0));
         s0 = extract_h(L_mult(s0, s0));
         e2 = energym_exp;
         t0 = energym_man;
         a2 = L_mult(t0, s0);
         e3 = norm_l(a1);
         t1 = extract_h(L_shl(a1, e3));
         a3 = L_mult(t1, cor2m);
         exp1 = shl(sub(exp0, cor2m_exp),1);
         exp1 = add(exp1, e2);
         exp1 = sub(exp1, e3);

         if (exp1>=0) 
            a2 = L_shr(a2, exp1);
         if (exp1<0) 
            a3 = L_shl(a3, exp1);
   
         IF (L_sub(a2, a3)>0) 
         {
            im = add(k,1);
            cor2m = s0;
            cor2m_exp = exp0;
            energym_exp = e3;
            energym_man = t1;
#if WMOPS
            move16();;move16();move16();move16();
#endif
	      }
      }        
    } 
    ELSE 
    {    /* if interpolated peak is on the left side */
      
      deltae = L_shr(L_sub(energy[n-1], eni), 3);
      FOR (k = 0; k < HDECF; k++) 
      {
	      a0=L_add(L_sub(Mpy_32_16(ah,al,x2[k]),Mpy_32_16(bh,bl,x[k])),cc);
	      eni = L_add(eni, deltae);
         a1 = L_max(eni, eni);

         exp0 = norm_l(a0);
         s0 = extract_h(L_shl(a0, exp0));
         s0 = extract_h(L_mult(s0, s0));
         e2 = energym_exp;
         t0 = energym_man;
         a2 = L_mult(t0, s0);
         e3 = norm_l(a1);
         t1 = extract_h(L_shl(a1, e3));
         a3 = L_mult(t1, cor2m);
         exp1 = shl(sub(exp0, cor2m_exp),1);
         exp1 = add(exp1, e2);
         exp1 = sub(exp1, e3);
         if (exp1>=0) 
            a2 = L_shr(a2, exp1);
         if (exp1<0) 
            a3 = L_shl(a3, exp1);
	
         IF (L_sub(a2, a3)>0) 
         {
            im = negate(add(k,1));
            cor2m = s0;
            cor2m_exp = exp0;
            energym_exp = e3;
            energym_man = t1;
#if WMOPS
            move16();move16();move16();move16();
#endif
         }
      }        
    }
    
    /* SEARCH DONE; ASSIGN cor2[] AND energy[] CORRESPONDING TO 
       INTERPOLATED PEAK */ 
    plag[i]=add(shl(add(idx[i],1),3),im); /* lag of interp. peak */
    cor2i[i]=cor2m;
    cor2i_exp[i]=cor2m_exp;
    /* interpolated energy[] of i-th interpolated peak */
    energyi_exp[i] = energym_exp;
    energyi_man[i] = energym_man;
#if WMOPS
    move16();move16();move16();move16();move16();
#endif
    
    /* SEARCH FOR GLOBAL MAXIMUM OF INTERPOLATED cor2[]/energy[] peak */
    a0 = L_mult(cor2m,energymax_man);
    a1 = L_mult(cor2max, energyi_man[i]);
    exp0 = shl(sub(cor2m_exp, cor2max_exp),1);
    exp0 = add(exp0, energymax_exp);
    exp0 = sub(exp0, energyi_exp[i]);

    if (exp0 >=0) 
      a0 = L_shr(a0, exp0);
    if (exp0<0) 
      a1 = L_shl(a1, exp0);

    IF (L_sub(a0,a1)>0) 
    {
      imax=i;
      cor2max=cor2m;
      cor2max_exp=cor2m_exp;
      energymax_exp = energyi_exp[i];
      energymax_man = energyi_man[i];
#if WMOPS
      move16();move16();move16();move16();move16();
#endif
    }
  }
  
  cpp=plag[imax];	/* first candidate for coarse pitch period */
  mplth=plag[sub(npeaks,1)]; /* set mplth to the lag of last peak */
#if WMOPS
  move16();move16();
#endif  

#if (DMEM)
    /* memory deallocation */
    deallocWord32(_cor, 0, MAXPPD1);
    deallocWord32(_energy, 0, MAXPPD1);
    deallocWord16(_energy_man, 0, MAXPPD1);
    deallocWord16(_energy_exp, 0, MAXPPD1);
    deallocWord16(_cor2, 0, MAXPPD1);
    deallocWord16(_cor2_exp, 0, MAXPPD1);
#endif
  
  /* FIND THE LARGEST PEAK (IF THERE IS ANY) AROUND THE LAST PITCH */
  maxdev= shr(cpplast,2); /* maximum deviation from last pitch */
  
  im = -1;
  cor2m=(Word16) 0x8000;
  cor2m_exp= (Word16) 0;
  energym_man = 1;
  energym_exp = 0;
#if WMOPS
  move16();move16();move16();move16();move16();
#endif
  FOR (i=0;i<npeaks;i++) 
  {  /* loop through the peaks before the largest peak */
    IF (sub(abs_s(sub(plag[i],cpplast)), maxdev)<=0) 
    {
      a0 = L_mult(cor2i[i],energym_man);
      a1 = L_mult(cor2m, energyi_man[i]);
      exp0 = shl(sub(cor2i_exp[i], cor2m_exp),1);
      exp0 = add(exp0, energym_exp);
      exp0 = sub(exp0, energyi_exp[i]);
      if (exp0 >=0) 
         a0 = L_shr(a0, exp0);
      if (exp0<0) 
         a1 = L_shl(a1, exp0);
      IF (L_sub(a0, a1)>0) 
      {
         im=i;
         cor2m=cor2i[i];
         cor2m_exp=cor2i_exp[i];
         energym_man = energyi_man[i];
         energym_exp = energyi_exp[i];
#if WMOPS
         move16();move16();move16();move16();move16();
#endif
      }	
    }
  } /* if there is no peaks around last pitch, then im is still -1 */
  
  /* NOW SEE IF WE SHOULD PICK ANY ALTERNATICE PEAK. */
  /* FIRST, SEARCH FIRST HALF OF PITCH RANGE, SEE IF ANY QUALIFIED PEAK
     HAS LARGE ENOUGH PEAKS AT EVERY MULTIPLE OF ITS LAG */
  i=0;
#if WMOPS
  move16();
#endif
  WHILE (sub(shl(plag[i],1), mplth)<0) 
  {
    
    /* DETERMINE THE APPROPRIATE THRESHOLD FOR THIS PEAK */
     t1 = sub(i,im);
    if (t1!=0) 
    {  /* if not around last pitch, */
      threshold = TH1;    /* use a higher threshold */
#if WMOPS
      move16();
#endif
    } 
    if (t1==0) 
    {        /* if around last pitch */
      threshold = TH2;    /* use a lower threshold */
#if WMOPS
      move16();
#endif
    }
    
    /* IF THRESHOLD EXCEEDED, TEST PEAKS AT MULTIPLES OF THIS LAG */
    a0 = L_mult(cor2i[i],energymax_man);
    t1 = extract_h(L_mult(energyi_man[i], threshold));
    a1 = L_mult(cor2max, t1);
    exp0 = shl(sub(cor2i_exp[i], cor2max_exp),1);
    exp0 = add(exp0, energymax_exp);
    exp0 = sub(exp0, energyi_exp[i]);
    if (exp0 >=0) a0 = L_shr(a0, exp0);

    if (exp0 <0) a1 = L_shl(a1, exp0);

    IF (L_sub(a0, a1)>0) 
    {
      flag=1;  
      j=add(i,1);
      k=0;
#if WMOPS
      move16();move16();
#endif
      s=shl(plag[i],1); /* initialize s to twice the current lag */
      WHILE (sub(s,mplth)<=0) 
      { /* loop thru all multiple lag <= mplth */
         mpflag=0;   /* initialize multiple pitch flag to 0 */
#if WMOPS
         move16();
#endif
         t0 = mult_r(s,MPDTH); 
         a=sub(s, t0);   /* multiple pitch range lower bound */
         b=add(s, t0);   /* multiple pitch range upper bound */
         FOR (;j<npeaks;j++)
         { /* loop thru peaks with larger lags */
            IF (sub(plag[j],b)>0) { /* if range exceeded, */
	            BREAK;          /* break the innermost loop */
         
            }       /* if didn't break, then plag[j] <= b */
            IF (sub(plag[j],a)>0) 
            { /* if current peak lag within range, */
               /* then check if peak value large enough */
               a0 = L_mult(cor2i[j],energymax_man);
               tt = sub(k,4);
               if (tt<0)
               {
                  t1 = MPTH[k];
#if WMOPS
                  move16();
#endif
               }
               if (tt>=0)
               {
                  t1 = MPTH4;
#if WMOPS
                  move16();
#endif
               }
               t1 = extract_h(L_mult(t1, energyi_man[j]));
               a1 = L_mult(cor2max, t1);
               exp0 = shl(sub(cor2i_exp[j], cor2max_exp),1);
               exp0 = add(exp0, energymax_exp);
               exp0 = sub(exp0, energyi_exp[j]);
               if (exp0 >=0) 
                  a0 = L_shr(a0, exp0);
               if (exp0<0)
                  a1 = L_shl(a1, exp0);
               IF (L_sub(a0,a1)>0) 
               {
                  mpflag=1; /* if peak large enough, set mpflag, */
#if WMOPS
                  move16();
#endif
                  BREAK; /* and break the innermost loop */
               } 
            }
         }
         /* if no qualified peak found at this multiple lag */
         IF (mpflag == 0) 
         { 
            flag=0;     /* disqualify the lag plag[i] */
#if WMOPS
            move16();
#endif
            BREAK;      /* and break the while (s-mplth<=0) loop */
         }
         k=add(k,1);
         s = add(s, plag[i]); /* update s to the next multiple pitch lag */
      }
      
      /* if there is a qualified peak at every multiple of plag[i], */
      IF (sub(flag,1)==0) 
      { 
         cpp = plag[i]; /* accept this as final coarse pitch period */
#if WMOPS
         move16();
#endif
#if (DMEM)
         /* memory deallocation */
         deallocWord16(plag, 0, HMAXPPD-1);
         deallocWord16(energyi_man, 0, HMAXPPD-1);
         deallocWord16(energyi_exp, 0, HMAXPPD-1);
         deallocWord16(cor2i, 0, HMAXPPD-1);
         deallocWord16(cor2i_exp, 0, HMAXPPD-1);
#endif

		   return cpp;         /* return to calling function */
      }
   }       
   i=add(i,1);
   IF (sub(i,npeaks)==0)
      BREAK;      /* to avoid out of array bound error */
  }
  
#if (DMEM)
  /* memory deallocation */
  deallocWord16(energyi_man, 0, HMAXPPD-1);
  deallocWord16(energyi_exp, 0, HMAXPPD-1);
  deallocWord16(cor2i, 0, HMAXPPD-1);
  deallocWord16(cor2i_exp, 0, HMAXPPD-1);
#endif

  /* IF PROGRAM PROCEEDS TO HERE, NONE OF THE PEAKS WITH LAGS < 0.5*mplth
     QUALIFIES AS THE FINAL COARSE PITCH PERIOD. IN THIS CASE, CHECK IF
     THERE IS ANY PEAK LARGE ENOUGH AROUND LAST COARSE PITCH PERIOD.  
     IF SO, USE ITS LAG AS THE FINAL COARSE PITCH PERIOD. */
  IF (add(im,1)!=0) 
  {   /* if there is at least one peak around last coarse pitch period */
     tt=sub(im, imax);
    IF (tt==0) 
    { /* if this peak is also the global maximum, */

#if (DMEM)
      /* memory deallocation */
      deallocWord16(plag, 0, HMAXPPD-1);
#endif

      return cpp;   /* return first pitch candidate at global maximum */
    }
    IF (tt<0) 
    { /* if lag of this peak < lag of global maximum, */
      a0 = L_mult(cor2m,energymax_man);
      t1 = extract_h(L_mult(energym_man, LPTH2));
      a1 = L_mult(cor2max, t1);
      exp0 = shl(sub(cor2m_exp, cor2max_exp),1);
      exp0 = add(exp0, energymax_exp);
      exp0 = sub(exp0, energym_exp);
      if (exp0 >=0) 
         a0 = L_shr(a0, exp0);
      if (exp0<0) 
         a1 = L_shl(a1, exp0);
      IF (L_sub(a0,a1)>0) 
      {
	      IF (sub(plag[im], i_mult(HMAXPPD,cpp_scale))>0) 
         {
            cpp = plag[im];
#if WMOPS
            move16();
#endif
#if (DMEM)
	         /* memory deallocation */
	         deallocWord16(plag, 0, HMAXPPD-1);
#endif

	  	      return cpp;
         }
	      FOR (k=2; k<=5;k++) 
         { /* check if current candidate pitch */
            s=mult(plag[imax],invk[k-2]); /* is a sub-multiple of */
            t0 = mult_r(s,SMDTH);
            a=sub(s, t0);  		/* the time lag of */
            b=add(s, t0);       /* the global maximum peak */
#ifdef WMOPS
            test();
#endif
            IF (sub(plag[im],a)>0 && sub(plag[im],b)<0) 
            {     /* if so, */
               cpp = plag[im];		/* accept this lag */
#if WMOPS
               move16();
#endif
#if (DMEM)
	            /* memory deallocation */
	            deallocWord16(plag, 0, HMAXPPD-1);
#endif
	            return cpp;         /* and return as pitch */
            }
         }
      }
    } 
    ELSE 
    {           /* if lag of this peak > lag of global max, */
      a0 = L_mult(cor2m,energymax_man);
      t1 = extract_h(L_mult(energym_man, LPTH1));
      a1 = L_mult(cor2max, t1);
      exp0 = shl(sub(cor2m_exp, cor2max_exp),1);
      exp0 = add(exp0, energymax_exp);
      exp0 = sub(exp0, energym_exp);
      if (exp0 >=0) 
         a0 = L_shr(a0, exp0);
      if (exp0<0) 
         a1 = L_shl(a1, exp0);
      IF (L_sub(a0,a1)>0) 
      {
#if WMOPS
         move16();
#endif
         cpp = plag[im];	/* accept its lag */
#if (DMEM)
	/* memory deallocation */
	deallocWord16(plag, 0, HMAXPPD-1);
#endif

		   return cpp;
      }
    }
  }
  
  /* IF PROGRAM PROCEEDS TO HERE, WE HAVE NO CHOICE BUT TO ACCEPT THE
     LAG OF THE GLOBAL MAXIMUM */

#if (DMEM)
  /* memory deallocation */
  deallocWord16(plag, 0, HMAXPPD-1);
#endif

  return cpp;
  
}
