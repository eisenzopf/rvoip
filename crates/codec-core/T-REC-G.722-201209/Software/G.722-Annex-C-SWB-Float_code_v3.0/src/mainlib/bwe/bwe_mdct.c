/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "pcmswb_common.h"
#include "bwe_mdct_table.h"
#include "bwe_mdct.h"

#include "floatutil.h"


void f_cfft(
  Float * x1,   /* (i/o) real part of data                 */
  Float * x2,   /* (i/o) imaginary part of data            */
  Short   sign  /* (i) flag to select FFT (1) or IFFT (-1) */
);


void f_cfft(
  Float * fx1,   /* (i/o) real part of data                 */
  Float * fx2,   /* (i/o) imaginary part of data            */
  Short   sign  /* (i) flag to select FFT (1) or IFFT (-1) */
)
{	
  Float    fACC0;                 /* first ACC */
  Float    fACC1;                 /* second ACC */

  Float *ftab_x1, *ftab_x2, *frx1, *frx2;

  Short mdct_np, mdct_npp, mdct_exp_npp, mdct_nb_rev;
  const Float *fmdct_rw1, *fmdct_rw2;
  const Short *mdct_tab_rev_i, *mdct_tab_rev_ipp;
  const Float *fmdct_xcos, *fmdct_xsin;

  Float *fptr_x1;               /* Pointer on tab_x1 */
  Float *fptr_x2;               /* Pointer on tab_x2 */
  Float *fptr0_x1;              /* Pointer on tab_x1 for DFT step */
  Float *fptr0_x2;              /* Pointer on tab_x2 for DFT step */
  const Float *fptr_cos;          /* Pointer on cos table */
  const Float *fptr_sin;          /* Pointer on sin table */
  const Short *ptr_map;          /* Pointer on mapping indice (input and output) */
  const Short *mdct_tab_map2;

  Short    ip, ipp, i, j;
  Float    fx1_tmp;
  Float    fx2_tmp;
  Short    exp_n2;
  Short    n1;
  Short    n2;                 /* size of sub array */
  Short    n3;
  Short    p;
  Short    inkw;
  Float    fw1;
  Float    fw2;
  Float    frix;
  Float    fcix;
  Float    frjx;
  Float    fcjx;
  Short    q;

  /* 80 points MDCT */
  mdct_np = MDCT2_NP;
  mdct_npp = MDCT2_NPP;
  mdct_exp_npp = MDCT2_EXP_NPP;
  mdct_nb_rev = MDCT2_NB_REV;

  ptr_map = MDCT_tab_map_swbs;

  fmdct_rw1 = MDCT_rw1_tbl_swbf;
  fmdct_rw2 = MDCT_rw2_tbl_swbf;

  mdct_tab_rev_i = MDCT_tab_rev_i_swbs;
  mdct_tab_rev_ipp = MDCT_tab_rev_ipp_swbs;

  fmdct_xcos = MDCT_xcos_swbf;
  fmdct_xsin = MDCT_xsin_swbf;
  mdct_tab_map2 = MDCT_tab_map2_swbs;

  ftab_x1 = (Float *) calloc ( (mdct_np * mdct_npp), sizeof(Float) );
  ftab_x2 = (Float *) calloc ( (mdct_np * mdct_npp), sizeof(Float) );
  frx1 = (Float *) calloc ( mdct_np, sizeof(Float) );
  frx2 = (Float *) calloc ( mdct_np, sizeof(Float) );


  /********************************************************************************
   * Re-indexing (mapping of input indices)                                       *
   ********************************************************************************/

  fptr_x1 = ftab_x1;
  fptr_x2 = ftab_x2;
  for (ip = 0; ip < mdct_np; ip++) {
    for (ipp = 0; ipp < mdct_npp; ipp++) {
      i = (Short) * ptr_map++;
      *fptr_x1++ = fx1[i];
      *fptr_x2++ = fx2[i];
    }
  }


  /*******************************************************************************/

  fptr_x1 = ftab_x1;
  fptr_x2 = ftab_x2;

  for (ip = 0; ip < mdct_np; ip++) {
    for (j = 0; j < mdct_nb_rev; j++) {
      i = mdct_tab_rev_i[j];
      ipp = mdct_tab_rev_ipp[j];
      fx1_tmp = fptr_x1[ipp];     /* swap value ptr_x1[i] and ptr_x1[ipp] */
      fx2_tmp = fptr_x2[ipp];     /* swap value ptr_x2[i] and ptr_x2[ipp] */
      fptr_x1[ipp] = fptr_x1[i];
      fptr_x2[ipp] = fptr_x2[i];
      fptr_x1[i] = fx1_tmp;
      fptr_x2[i] = fx2_tmp;
    }
    fptr_x1 += mdct_npp;
    fptr_x2 += mdct_npp;
  }

  /*******************************************************************************
   * n1 size of butterfly                                                        *
   *******************************************************************************/

  fptr_x1 = ftab_x1;
  fptr_x2 = ftab_x2;

  /* 80 points MDCT */
  for (ip = 0; ip < mdct_np; ip++)
  {
	  for (exp_n2 = 0; exp_n2 <= mdct_exp_npp; exp_n2++)
	  {
		  n2 = 1 << exp_n2;
		  n1 = n2 >> 1;
		  n3 = mdct_exp_npp - exp_n2;
		  inkw = 0;
		  q = 1 << n3;

		  for (p = 0; p < n1; p++)
		  {
			  /* get twiddle factor in arrays rw1 and rw2 */
			  fw1 = fmdct_rw1[inkw];
			  fw2 = fmdct_rw2[inkw];
			  inkw += q; /* q is constant inside the loop */

			  if (sign > 0)
			  {
				  fw2 = -fw2;
			  }

			  if (sign > 0)            /* FFT */
			  {
				  for (i = p; i < mdct_npp; i += n2)
				  {
					  /* select item p in array p */
					  j = i + n1;

					  /* butterfly on x[i] and x[j] */
					  frix = fptr_x1[i];
					  fcix = fptr_x2[i];

					  /* twiddle factor */
                      fACC0 = (fw1 * fptr_x1[j]) - (fw2 * fptr_x2[j]);
                      fACC1 = (fw2 * fptr_x1[j]) + (fw1 * fptr_x2[j]);

					  frjx = fACC0;
					  fcjx = fACC1;

                      fptr_x1[i] = frix + frjx;
                      fptr_x2[i] = fcix + fcjx;
                      fptr_x1[j] = frix - frjx;
                      fptr_x2[j] = fcix - fcjx;
				  }
			  }
			  else                    /* IFFT */
			  {
				  for (i = p; i < mdct_npp; i += n2)
				  {
					  /* select item p in array p */
					  j = i + n1;

					  /* butterfly on x[i] and x[j] */
					  frix = fptr_x1[i]/* / 2.0f*/;
					  fcix = fptr_x2[i]/* / 2.0f*/;

					  /* twiddle factor */
			          fACC0 = (fw1 * fptr_x1[j]) - (fw2 * fptr_x2[j]);
					  frjx = fACC0/* / 2.0f*/;
					  fACC0 = (fw2 * fptr_x1[j]) + (fw1 * fptr_x2[j]);
					  fcjx = fACC0/* / 2.0f*/;

					  fptr_x1[i] = frix + frjx;
					  fptr_x2[i] = fcix + fcjx;
					  fptr_x1[j] = frix - frjx;
					  fptr_x2[j] = fcix - fcjx;
				  }
			  }
		  }
	  }                           /* end while */
	  fptr_x1 += mdct_npp;
	  fptr_x2 += mdct_npp;
  }                             /* end for ip */ 

  /**************************************************************************/

  fptr0_x1 = ftab_x1;
  fptr0_x2 = ftab_x2;

  for (ipp = 0; ipp < mdct_npp; ipp++)
  {
    fptr_x1 = fptr0_x1;
    fptr_x2 = fptr0_x2;
    for (ip = 0; ip < mdct_np; ip++)
    {
      frx1[ip] = *fptr_x1;
      frx2[ip] = *fptr_x2;

      fptr_x1 += mdct_npp;
      fptr_x2 += mdct_npp;
    }
    fptr_x1 = fptr0_x1++;
    fptr_x2 = fptr0_x2++;

    fptr_cos = fmdct_xcos;
    fptr_sin = fmdct_xsin;
    if (sign > 0)                /* FFT */
    {
      for (ip = 0; ip < mdct_np; ip++)
      {
        /* Set Overflow to 0 to test it after radix 5 with Q15 sin & cos coef */
        /* keep pointer's position on cos & sin tables */

        fACC0 = 0.0f;
        fACC1 = 0.0f;

        for (i = 0; i < mdct_np; i++)
        {
          fACC0 += ((frx1[i] * (*fptr_cos)) -  (frx2[i] * (*fptr_sin)));
          fACC1 += ((frx2[i] * (*fptr_cos++)) + (frx1[i] * (*fptr_sin++)));
        }
        *fptr_x1 = fACC0 * 2.0f;
        *fptr_x2 = fACC1 * 2.0f;

        fptr_x1 += mdct_npp;
        fptr_x2 += mdct_npp;
      }
    }

	else                        /* IFFT */
    {
      for (ip = 0; ip < mdct_np; ip++)
      {

        /* Set Overflow to 0 to test it after radix 5 with Q15 sin & cos coef */
        /* keep pointer's position on cos & sin tables */

        fACC0 = 0.0f;
        fACC1 = 0.0f;

        for (i = 0; i < mdct_np; i++)
        {
          fACC0 += ((frx1[i] * (*fptr_cos)) + (frx2[i] * (*fptr_sin)));
		  fACC1 += ((frx2[i] * (*fptr_cos++)) - (frx1[i] * (*fptr_sin++)));
        }
        *fptr_x1 = fACC0 / 2.0f;
        *fptr_x2 = fACC1 / 2.0f;

        fptr_x1 += mdct_npp;
        fptr_x2 += mdct_npp;
      }                         /* end for ip */
    }
  }                             /* end for ipp */


  /***************************************************************************
   * mapping for the output indices                                          *
   ***************************************************************************/
  fptr_x1 = ftab_x1;
  fptr_x2 = ftab_x2;
  ptr_map = mdct_tab_map2;

  for (ip = 0; ip < mdct_np; ip++)
  {
    for (ipp = 0; ipp < mdct_npp; ipp++)
    {
      i = (Short) * ptr_map++;
      fx1[i] = *fptr_x1++;
      fx2[i] = *fptr_x2++;
    }
  }
  free(ftab_x1);
  free(ftab_x2);
  free(frx1);
  free(frx2);

  return;
}



void f_PCMSWB_TDAC_inv_mdct(
  Float * xr,         /* (o):   output samples                     */
  Float * ykq,        /* (i):   MDCT coefficients                  */
  Float * ycim1,      /* (i):   previous MDCT memory               */
  Short   loss_flag,  /* (i):   packet-loss flag                   */
  Float * cur_save   /* (i/o): signal saving buffer               */		
)
{
  Float   fACC0;
  Float   fACC1;

  Float   *fycr, *fyci, *fsig_cur, *fsig_next;
  Short   mdct_l_win, mdct_l_win2, mdct_l_win4;
  const Float   *fmdct_h;
  const Float   *fmdct_wetrm1, *fmdct_wetim1;
  const Float   *fptr_wsin, *fptr_wcos;
  const Float   *fptr_h;         /* Pointer on window */

  Short   k;
  Float   tmpF;

  Float   *fptr_yci;
  Float   *fptr_ycr;
  Float   *fptr1;
  Float   *fptr2;
  Float   *fptr1_next;
  Float   *fptr2_next;
  
  /* 80 points MDCT */
  mdct_l_win  = MDCT2_L_WIN;
  mdct_l_win2 = MDCT2_L_WIN2;
  mdct_l_win4 = MDCT2_L_WIN4;

  fmdct_h = MDCT_h_swbf;

  fmdct_wetrm1 = MDCT_wetrm1_swbf;
  fmdct_wetim1 = MDCT_wetim1_swbf;

  fptr_wsin = MDCT_wsin_swbf;
  fptr_wcos = MDCT_wsin_swbf + mdct_l_win4;
  
  /* Higher-band frame erasure concealment (FERC) in time domain */
  if (loss_flag != 0)
  {
	  fptr_h = fmdct_h + mdct_l_win2;
	  for (k = 0; k < mdct_l_win2; k++)
	  {
		  cur_save[k] = 0.875f/*ATT_FEC_COEF*/ * cur_save[k];

		  fACC0 = cur_save[k] * fmdct_h[k];
		  fACC0 += ycim1[k] * (*(--fptr_h));
		  xr[k] = fACC0;

		  ycim1[k] = 0.875f/*ATT_FEC_COEF*/ * ycim1[k];
	  }
	  return;
  }

  fycr = (Float *) calloc (mdct_l_win4, sizeof(Float));
  fyci = (Float *) calloc (mdct_l_win4, sizeof(Float));
  fsig_cur = (Float *) calloc (mdct_l_win2, sizeof(Float));
  fsig_next = (Float *) calloc (mdct_l_win2, sizeof(Float));


  /*******************************************************************************/
  /* Inverse MDCT computation                                                    */
  /*******************************************************************************/

  /* 1 --> Input rotation = Product by wetrm1 */

  fptr1 = ykq;
  fptr2 = ykq + (mdct_l_win2 - 1); 
  fptr_yci = fyci;
  fptr_ycr = fycr;

  for (k = 0; k < mdct_l_win4; k++)
  {
    fACC0 = *fptr2 * fmdct_wetrm1[k];
    fACC0 = -fACC0;
    fACC0 -= *fptr1 * fmdct_wetim1[k];
    *fptr_ycr++ = fACC0;

    fACC0 = *fptr1++ * fmdct_wetrm1[k];
    fACC0 -= *fptr2-- * fmdct_wetim1[k];
    *fptr_yci++ = fACC0;
    fptr1++;
    fptr2--;
  }

  /* 2 --> Forward FFT : size = 20 */
  f_cfft(fycr, fyci, 1);

  /* 3 --> Output rotation : product by a complex exponent */

  fptr_yci = fyci;
  fptr_ycr = fycr;

  for (k = 0; k < mdct_l_win4; k++)
  {
    tmpF = *fptr_ycr;


    fACC0 = tmpF * *fptr_wcos;
	fACC0 += fyci[k] * (*fptr_wsin);
    *fptr_ycr++ = fACC0;

    fACC0 = fyci[k] * (*fptr_wcos);
    fACC0 -= tmpF * (*fptr_wsin);
    *fptr_yci++ = fACC0;

	fptr_wsin++;
	fptr_wcos--;

  }

  /* 4 --> Overlap and windowing (in one step) - equivalent to complex product */

  fptr1 = fsig_cur;
  fptr2 = fsig_cur + mdct_l_win2 - 1; 
  fptr1_next = fsig_next;
  fptr2_next = fsig_next + mdct_l_win2 - 1; 

  for (k = 0; k < mdct_l_win4; k++)
  {
    *fptr1++ = fycr[k];
    *fptr2-- = -fycr[k];
	*fptr1_next++ = fyci[k];
    *fptr2_next-- = fyci[k];

    fptr1++;
    fptr1_next++;
    fptr2--;
    fptr2_next--;
  }

  fptr_h = fmdct_h + mdct_l_win2; 
  for (k = 0; k < mdct_l_win2; k++)
  {
    fACC0  = fsig_cur[k] *fmdct_h[k];
    fACC1  = ycim1[k] * (*(--fptr_h));
    fACC0  = fACC0 + fACC1;
    xr[k] = fACC0;
	ycim1[k] = fsig_next[k];
  }

  /* Save sig_cur for FERC */
  movF(mdct_l_win2, fsig_cur, cur_save);

  free(fycr);
  free(fyci);
  free(fsig_cur);
  free(fsig_next);

  return;
}



void f_bwe_mdct(
  Float * f_mem,        /* (i): old input samples    */
  Float * f_input,      /* (i): input samples        */
  Float * f_ykr,        /* (o): MDCT coefficients    */
  Short mode		   /* (i): mdct mode (0: 40-points, 1: 80-points) */
)
{
  Float f_ACC0;               /* ACC */

  Float   *fxin, *fycr, *fyci;
  Short   mdct_l_win, mdct_l_win2, mdct_l_win4;
  const Float *fptr_h1;        /* pointer on window */
  const Float *fptr_h2;        /* pointer on window */
  Float   *fptr_x1;            /* pointer on input samples */
  Float   *fptr_x2;            /* pointer on input samples */
  Float   *fptr_ycr;           /* pointer on ycr */
  Float   *fptr_yci;           /* pointer on yci */

  Float *fptr_x3, *fptr_x4;
  const Float   *fptr_wsin, *fptr_wcos;
  const Float   *fptr_weti, *fptr_wetr;

  Short k,i;
  Float tmpF_ycr;


  /* 80 points MDCT */
  mdct_l_win = MDCT2_L_WIN;
  mdct_l_win2 = MDCT2_L_WIN2;
  mdct_l_win4 = MDCT2_L_WIN4;

  fptr_h1 = MDCT_h_swbf;                          /* Start of the window */
  fptr_h2 = MDCT_h_swbf + mdct_l_win2 - 1; /* End of the window   */	
 
  fptr_wsin = MDCT_wsin_swbf;                      
  fptr_wcos = MDCT_wsin_swbf + mdct_l_win4; 

  fptr_weti = MDCT_weti_swbf;
  fptr_wetr = MDCT_wetr_swbf;

  fxin = (Float *) calloc ( mdct_l_win, sizeof(Float) );
  fycr = (Float *) calloc ( mdct_l_win4, sizeof(Float) );
  fyci = (Float *) calloc ( mdct_l_win4, sizeof(Float) );

  /********************************************************************************/
  /* MDCT Computation                                                             */
  /********************************************************************************/

  /* form block of length N */
  movF( mdct_l_win2 , f_mem , fxin );
  movF( mdct_l_win2 , &f_input[0] , &fxin[mdct_l_win2]);
  /* Step 1 --> Pre-scaling of input signal
                compute norm_shift               */


   /* Step 2 --> Calculate zn =  (y2n-yN/2-1-2n) + j(yN-1-2n+yN+2+2n) (complex terms), 
                          for n=0...N/4-1                                         */

  {
	fptr_x1 = fxin;

    fptr_x2 = fxin + mdct_l_win2 - 1;	
    fptr_x3 = fxin + mdct_l_win2;	
    fptr_x4 = fxin + mdct_l_win - 1;	

    fptr_yci = fyci;
    fptr_ycr = fycr;

    for (i = 0; i < mdct_l_win4; i++)
    {
      *fptr_ycr++ = (*fptr_h1 * (*fptr_x1)) - (*fptr_h2 * (*fptr_x2));
	  *fptr_yci++ = (*fptr_h1 * (*fptr_x4)) + (*fptr_h2 * (*fptr_x3));

      fptr_h1 += 2;	
      fptr_h2 -= 2;	
      fptr_x1 += 2;	
      fptr_x2 -= 2;	
      fptr_x3 += 2;	
      fptr_x4 -= 2;	
    }
  }

  /* Step 3 --> Calculate z'n = zn.WN^n, for n=0...N/4-1 */
  fptr_yci = fyci;
  fptr_ycr = fycr;

  for(k = 0; k < mdct_l_win4; k++)
  {
	tmpF_ycr = *fptr_ycr;

    f_ACC0 = tmpF_ycr * (*fptr_wcos);
    f_ACC0 -= (fyci[k] * (*fptr_wsin));
    *fptr_ycr++ = f_ACC0;

    f_ACC0 = tmpF_ycr * (*fptr_wsin);
    f_ACC0 += (fyci[k] * (*fptr_wcos));
    *fptr_yci++ = f_ACC0;

	fptr_wsin++;
    fptr_wcos--;
  }


  /* Step 3 --> Inverse FFT of size N/4: Z'k = FFT-1 z'n, for k=0...N/4-1 */
  f_cfft(fycr, fyci, -1);

  /* Step 4 --> Calculate Zk = 1/80 . ((-1)^k+1.W8^-1.W4N^(4k+1)) . Z'k
  
     Step 5 --> Rearranging results:
                     Y2k       = Im[Zk]
                     Y2(k+N/4) = Re[Zk]

     Since Y2(k+N/4) =-Y(N/2-1-2k), results are actually presented as follows:
                     Y2k       = Im[Zk]
                     YN/2-1-2k = -Re[Zk]                                             
       
     Steps 4 & 5 are integrated below in a single step */

  fptr_x1 = f_ykr;
  fptr_x2 = f_ykr + mdct_l_win2 - 1;	

  for (k = 0; k < mdct_l_win4; k++)
  {
    tmpF_ycr = fycr[k];

    /* symetry of coeff k-1 and N-k */

    f_ACC0 = (fyci[k] * fptr_weti[k]) - (tmpF_ycr * fptr_wetr[k]);
    *fptr_x2-- = f_ACC0;
    fptr_x2--;

    f_ACC0 = (tmpF_ycr * fptr_weti[k]) + (fyci[k] * fptr_wetr[k]);
    *fptr_x1++ = f_ACC0;
    fptr_x1++;
  }

  /* Step 6 --> Post-scaling of MDCT coefficient
                compute norm_shift               */

  /* update memory */
  movF(mdct_l_win2, f_input, f_mem);

  free(fxin);
  free(fycr);
  free(fyci);

  return;
}                               /* END MDCT */
