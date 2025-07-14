/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/* Include state variable definition, function and operator prototypes */
#include "funcg722.h"


static Short  saturate2(
                long     x,     /* (i): input value   */
                Short  x_min,    /* (i): lower limit   */
                Short  x_max     /* (i): higher limit   */
                );



static void adpcm_adapt_c(Short ind, Short *a, Short *b, Short *d, Short *p, Short *r,
                          Short *nb, Short *det, Short *sz, Short *s)
{
  Short          sp;
  long tmp32;

  tmp32 = (long)d[0] + (long)(*sz);
  p[0] = saturate2(tmp32, -32768, 32767);  	/* parrec */
  tmp32 = (long)*s + (long)d[0];
  r[0] = saturate2(tmp32, -32768, 32767);   /* recons */

  upzero (d, b);
  uppol2 (a, p);
  uppol1 (a, p);
  *sz = filtez (d, b); 
  sp = filtep (r, a);
  
  tmp32 = (long)sp + *sz;
  *s = saturate2(tmp32, -32768, 32767);     /* predic */
  return;
} 

void adpcm_adapt_h(Short ind, Short *a, Short *b, Short *d, Short *p, Short *r,
                   Short *nb, Short *det, Short *sz, Short *s)
{
  d[0] = (Short)(((long)*det * (long)qtab2[ind]) >> 15); 
  *nb = logsch (ind, *nb); 
  *det = scaleh (*nb); 
  adpcm_adapt_c(ind, a, b, d, p, r, nb, det, sz, s);
  return;
}

void adpcm_adapt_l(Short ind, Short *a, Short *b, Short *d, Short *p, Short *r,
                   Short *nb, Short *det, Short *sz, Short *s)
{
  d[0] = (Short)(((long)(*det) * (long)qtab4[ind >> 2]) >> 15); 
  *nb = logscl (ind, *nb); 
  *det = scalel (*nb); 
  adpcm_adapt_c(ind, a, b, d, p, r, nb, det, sz, s);
  return;
}





/*___________________________________________________________________________

Function Name : lsbdec                                                  

Purpose :                                                               

Decode lower subband of incomung speech/music.                         

Inputs :                                                                
ilr - ADPCM encoding of the low sub-band                              
mode - G.722 operation mode                                           
s   - pointer to state variable (read/write)                          

Return Value :                                                          
Decoded low-band portion of the recovered sample as a 16-bit word      
___________________________________________________________________________
*/
#define AL   s->al
#define BL   s->bl
#define DETL s->detl
#define DLT  s->dlt
#define NBL  s->nbl
#define PLT  s->plt
#define RLT  s->rlt
#define SL   s->sl
#define SPL  s->spl
#define SZL  s->szl



Short lsbdec (Short ilr, Short mode, g722_state *s)
{
  Short  dl, rl, yl;
  long tmp32;

   tmp32 =((long)DETL * (long)(invqbl_tab[mode][ilr >> invqbl_shift[mode]]) ) >> 15;
  dl = (Short)tmp32;
  tmp32 += (long)SL;
  rl = saturate2(tmp32, -32768, 32767);
  yl = limit (rl);

  adpcm_adapt_l(ilr, AL, BL, DLT, PLT, RLT, &NBL, &DETL, &SZL, &SL);
  return (yl);
}




#undef AL
#undef BL
#undef DETL
#undef DLT
#undef NBL
#undef PLT
#undef RLT
#undef SL
#undef SPL
#undef SZL
/* ........................ End of lsbdec() ........................ */



/*___________________________________________________________________________
Function Name : quantl5b                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short quantl5b (Short el, Short detl)
{
  Short  sil, mil, val, wd; 
  sil = el >> 15;
  wd = 32767 - (el & 32767);
  if(sil == 0)
  {
    wd = el;
  }

  val = (Short)(((long)6288 * (long)detl) >> 15);
  mil = 3;
  if ( (wd-val) >= 0)
  {
    mil = 11;
  }

  val = (Short)( ((long)q5b[mil]* (long)detl)>>15);
  mil -= 2;
  if ((wd-val) >= 0)
  {
    mil += 4;
  }

  val = (Short)( ((long)q5b[mil]* (long)detl)>>15);
  mil -= 1;
  if ((wd-val) >= 0)
  {
    mil += 2;
  }

  val = (Short)( ((long)q5b[mil]* (long)detl)>>15);
  
  if ((wd-val) >= 0)
  {
    mil += 1;
  }
  if(mil >14) 
  {
	  mil = 14;
  }

  if(sil == 0)
  {
    mil += 15;
  }

  return (misil5b[mil]);
}



/* ..................... End of quantl5b() ..................... */


/*___________________________________________________________________________

Function Name quanth:                                                   

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short quanth (Short eh, Short deth)
{
  Short          sih, mih, wd;

  sih = eh>> 15;
  wd = 32767 - (eh & 32767);
  if(sih == 0)
  {
    wd = eh;
  }
  mih = 1;
  if ( (wd - (Short)(( (long)q2* (long)deth )>>15) )>= 0)
  {
    mih = 2;
  }

  sih += 1;

  return (misih[sih][mih]);
}



/* ..................... End of quanth() ..................... */


/*___________________________________________________________________________

Function Name : filtep                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short filtep (Short rlt [], Short al [])
{

  Short  wd1, wd2, spl;
  long tmp32;

  /* shift of rlt */
  rlt[2] = rlt[1];  
  rlt[1] = rlt[0]; 

  tmp32 = (long)rlt[1] ;
  tmp32 = ((long)al[1] * tmp32 )>> 14;
  wd1 = saturate2(tmp32, -32768, 32767);
  tmp32 = (long)rlt[2] ;
  tmp32 = ((long)al[2] * tmp32) >> 14;
  wd2 = saturate2(tmp32, -32768, 32767);
  spl = wd1 + wd2;

  return (spl);
}




/* ..................... End of filtep() ..................... */


/*___________________________________________________________________________

Function Name : filtez                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short filtez (Short dlt [], Short bl [])
{
  Short  szl;
  Short  i;
  long tmp32;

  szl = 0;
  for (i = 6; i > 0; i--)
  {
    tmp32 = (long)dlt[i];
    tmp32 = (tmp32 * (long)bl[i])>>14;
	tmp32 += (long)szl ;
	szl = saturate2(tmp32, -32768, 32767);
  }

  return (szl);
}




/* ..................... End of filtez() ..................... */


/*___________________________________________________________________________

Function Name : limit                                                   

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/


Short limit (Short rl)
{
	if( rl >= -16384)
		rl = rl;
    else
		rl = -16384;
    if( rl <= 16383)
		rl = rl;
    else
		rl = 16383;
	return (rl);
}




/* ..................... End of limit() ..................... */


/*___________________________________________________________________________

Function Name : logsch                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short logsch (Short ih, Short nbh)
{
	Short nbph;
	nbph = (Short)(((long)nbh* (long)32512)>>15) + whi[ih];

	if( nbph >= 0)
		nbph = nbph;
	else
		nbph = 0;
	if( nbph <= 22528)
		nbph = nbph;
	else
		nbph = 22528;
  
  return (nbph);
}




/* ..................... End of logsch() ..................... */


/*___________________________________________________________________________

Function Name : logscl                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/


Short logscl (Short il, Short nbl)
{
  Short  ril, nbpl;
  ril = il >> 2;
  nbpl = (Short)(((long)nbl* (long)32512)>>15) + wli[ril];
  if( nbpl >= 0)
	  nbpl = nbpl;
  else
	  nbpl = 0;
  if( nbpl <= 18432)
	  nbpl = nbpl;
  else
	  nbpl = 18432;

  return (nbpl);
}



/* ..................... End of logscl() ..................... */


/* ************** Table ILA used by scalel and scaleh ******************** */

/* ************************* End of Table ILA **************************** */


/*___________________________________________________________________________

Function Name : scalel                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short scalel (Short nbpl)
{
  Short  wd1, wd2;
  wd1 = (nbpl>> 6) & 511;
  wd2 = wd1+ 64;
  return (ila2[wd2]);
}


/* ..................... End of scalel() ..................... */


/*___________________________________________________________________________

Function Name : scaleh                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          

___________________________________________________________________________
*/

Short scaleh (Short nbph)
{
  Short wd;
  wd = (nbph>> 6) & 511;
  return (ila2[wd]);
}


/* ..................... End of scaleh() ..................... */


/*___________________________________________________________________________

Function Name : uppol1                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          
None.                                                                  
___________________________________________________________________________
*/

void uppol1 (Short al [], Short plt [])
{
  Short   sg0, sg1, wd1, wd3, apl1;
  long tmp32;
  sg0 = plt[0]>> 15;
  sg1 = plt[1]>> 15;
  wd1 = -192;
  if((sg0- sg1) == 0)
  {
    wd1 = 192;
  }
  tmp32 = (long)(al[1]* (long)32640)>>15;
  tmp32 += (long)wd1;
  apl1 = saturate2(tmp32, -32768, 32767);
  wd3 = 15360- al[2];
  if( apl1 >= -wd3)
	  apl1 = apl1;
  else
	  apl1 = -wd3;
  if( apl1 <= wd3)
	  apl1 = apl1;
  else
	  apl1 = wd3;
  
  /* Shift of the plt signals */
  plt[2] = plt[1];
  plt[1] = plt[0];
  al[1] = apl1;
  return; 
}




/* ..................... End of uppol1() ..................... */


/*___________________________________________________________________________

Function Name : uppol2                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          
None.                                                                  
___________________________________________________________________________
*/

void uppol2 (Short al[], Short plt[])
{
  Short  sg0, sg1, sg2, wd1, wd2, wd3, wd4, wd5, apl2;
  long tmp32;

  sg0 = plt[0]>> 15;
  sg1 = plt[1]>> 15;
  sg2 = plt[2]>> 15;
  tmp32 = (long)al[1]<<2;
  wd1 = saturate2(tmp32, -32768, 32767);
  wd2 = wd1;
  if(sg0== sg1)
  {
    wd2 = saturate2(-tmp32, -32768, 32767);
  }
  wd2 = wd2>> 7;
  wd3 = -128;
  if(sg0==sg2)
  {
    wd3 = 128;
  }
  wd4 = wd2+ wd3;
  wd5 = (Short)( ((long)al[2]*(long)32512)>>15);
  apl2 = wd4+ wd5;
  if( apl2 >= -12288)
     apl2 = apl2;
  else
     apl2 = -12288;
  if( apl2 <= 12288)
     al[2] = apl2;
  else
     al[2] = 12288;

}




/* ..................... End of uppol2() ..................... */


/*___________________________________________________________________________

Function Name : upzero                                                  

Purpose :                                                               

.                  

Inputs :                                                                


Outputs :                                                               

none                                                                   

Return Value :                                                          
None.                                                                  

___________________________________________________________________________
*/


void upzero (Short dlt [], Short bl [])
{

  Short          sg0, sgi, wd1, wd2, wd3;
  Short          i;
  /* shift of the dlt line signal and update of bl */
  wd1 = 128;
  if(dlt[0] == 0)
  {
    wd1 = 0;
  }
  sg0 = dlt[0]>> 15;

  for (i = 6; i > 0; i--)
  {
    sgi = dlt[i]>> 15;
    wd3 = (Short) ( (long)(bl[i]*(long)32640)>>15);
    wd2 = wd3- wd1;
    if((sg0- sgi)== 0)
    {
      wd2 = wd3+ wd1;
    }
    bl[i] = wd2;
    dlt[i] = dlt[i - 1];
  }
  return;
}



/* ..................... End of upzero() ..................... */


/* **** Coefficients for both transmission and reception QMF **** */
/* ..................... End of table coef_qmf[] ..................... */

/*___________________________________________________________________________

Function Name : qmf_tx                                                  

Purpose :                                                               

G722 QMF analysis (encoder) filter. Uses coefficients in array         
coef_qmf[] defined above.                                              

Inputs :                                                                
xin0 - first sample for the QMF filter (read-only)                     
xin1 - secon sample for the QMF filter (read-only)                     
xl   - lower band portion of samples xin0 and xin1 (write-only)        
xh   - higher band portion of samples xin0 and xin1 (write-only)       
s    - pointer to state variable structure (read/write)                

Return Value :                                                          
None.                                                                  
___________________________________________________________________________
*/

void fl_qmf_tx_buf (Short **xin, Short *xl, Short *xh, Short **delayx)
{

  /* Local variables */
  int i;
  Float          accuma, accumb;
  const Float          *pcoef;
  Short *pdelayx;
  /* Saving past samples in delay line */
  *--(*delayx) = *(*xin)++;
  *--(*delayx) = *(*xin)++;

  /* QMF filtering */
  pcoef = fl_coef_qmf;
  pdelayx = *delayx;
  
  accuma = (*pcoef++) * (Float)(*pdelayx++);
  accumb = (*pcoef++) * (Float)(*pdelayx++);
  for (i = 1; i < 12; i++)
  {
    accuma += (*pcoef++) * (Float)(*pdelayx++);
    accumb += (*pcoef++) * (Float)(*pdelayx++);
  }

  *xl = (Short)(Floor(accuma + accumb));
  *xh = (Short)(Floor(accuma - accumb));
  return;
}

/* ..................... End of qmf_tx_buf() ..................... */




/*___________________________________________________________________________

Function Name : qmf_rx_buf                                                  

G722 QMF synthesis (decoder) filter, whitout memory shift. 
Uses coefficients in array coef_qmf[] defined above.                                              

Inputs :                                                                
out      - out of the QMF filter (write-only)                
rl       - lower band portion of a sample (read-only)                     
rh       - higher band portion of a sample (read-only)                    
*delayx  - pointer to delay line allocated outside               

Return Value :                                                          
None.                                                                  
___________________________________________________________________________
*/

void  fl_qmf_rx_buf (Short rl, Short rh, Short **delayx, Short **out)
{
  int          i;
  Float          accuma, accumb;
  const Float *pcoef;
  Short *pdelayx;
  /* compute sum and difference from lower-band (rl) and higher-band (rh) signals */
  /* update delay line */
  *--(*delayx) = rl+ rh;
  *--(*delayx) = rl- rh;

  /* qmf_rx filtering */
  pcoef = fl_coef_qmf;
  pdelayx = *delayx;

  accuma = *pcoef++ * (Float)*pdelayx++;
  accumb = *pcoef++ *  (Float)*pdelayx++;
  for (i = 1; i < 12; i++)
  {
    accuma += *pcoef++ *  (Float)*pdelayx++;
    accumb += *pcoef++ * (Float)*pdelayx++;
  }

  	/* re-scale in the good range */
	accuma *= (Float)8.;
	accumb *= (Float)8.;

	/* computation of xout1 and xout2 */
	*(*out)++ =  (accuma > 32767.) ? 32767 : ( (accuma<-32768.) ? -32768 : (Short)(Floor(accuma)) );
	*(*out)++ =  (accumb > 32767.) ? 32767 : ( (accumb<-32768.) ? -32768 : (Short)(Floor(accumb)) );
  return;
}



/*----------------------------------------------------------------
Function:
Bounds a 32-bit value between x_min and x_max (Short).
Return value
the short bounded value
----------------------------------------------------------------*/
static Short  saturate2(
                long     x,     /* (i): input value   */
                Short  x_min,    /* (i): lower limit   */
                Short  x_max     /* (i): higher limit   */
                )
{
	Short xs;
	if(x < x_min) {
		xs= x_min;
	}
	else 
	{
		if(x > x_max) 
		{
			xs= x_max;
		}
		else 
			xs= (Short)x;
	}
	return(xs);
}


/* ******************** End of funcg722.c ***************************** */
