/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*                     v3.0 beta - 23/Aug/2006
============================================================================

                     U    U   GGG    SSSS  TTTTT
                     U    U  G       S       T
                     U    U  G  GG   SSSS    T
                     U    U  G   G       S   T
                      UUUU    GG     SSS     T

               ========================================
                ITU-T - USER'S GROUP ON SOFTWARE TOOLS
               ========================================
                                   

    =============================================================
    COPYRIGHT NOTE: This source code, and all of its derivations,
    is subject to the "ITU-T General Public License". Please have
    it  read  in    the  distribution  disk,   or  in  the  ITU-T
    Recommendation G.191 on "SOFTWARE TOOLS FOR SPEECH AND  AUDIO
                          CODING STANDARDS".
     ** This code has  (C) Copyright by CNET Lannion A TSS/CMC **
     ** (now France-Telecom Orange)                            **
    =============================================================


MODULE:         G722.C 7kHz ADPCM AT 64 KBIT/S MODULE ENCODER AND 
DECODER FUNCTIONS

ORIGINAL BY:
J-P PETIT 
CNET - Centre Lannion A
LAA-TSS                         Tel: +33-96-05-39-41
Route de Tregastel - BP 40      Fax: +33-96-05-13-16
F-22301 Lannion CEDEX           Email: petitjp@lannion.cnet.fr
FRANCE

History:
~~~~~~~~
14.Mar.95  v1.0       Released for use ITU-T UGST software package Tool 
                      based on the CNET's 07/01/90 version 2.00
01.Jul.95  v2.0       Changed function declarations to work with many compilers;
                      reformated <simao@ctd.comsat.com>
23.Aug.06  v3.0 beta  Updated with STL2005 v2.2 basic operators and G.729.1 methodology
                      <{balazs.kovesi,stephane.ragot}@orange-ft.com>
============================================================================
*/

/* Include state variable definition, function and operator prototypes */
#include "funcg722.h"
#include "dsputil.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

static void adpcm_adapt_c(Word16 ind, Word16 *a, Word16 *b, Word16 *d, Word16 *p, Word16 *r,
                          Word16 *nb, Word16 *det, Word16 *sz, Word16 *s)
{
  Word16          sp;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  p[0] = add (d[0], *sz); move16();   /* parrec */
  r[0] = add (*s, d[0]); move16();    /* recons */
  upzero (d, b);
  uppol2 (a, p);
  uppol1 (a, p);
  *sz = filtez (d, b); move16();
  sp = filtep (r, a);
  *s = add (sp, *sz); move16();        /* predic */

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
} 

void adpcm_adapt_h(Word16 ind, Word16 *a, Word16 *b, Word16 *d, Word16 *p, Word16 *r,
                   Word16 *nb, Word16 *det, Word16 *sz, Word16 *s)
{
  d[0] = mult(*det, qtab2[ind]); move16();
  *nb = logsch (ind, *nb); move16();
  *det = scaleh (*nb); move16();
  adpcm_adapt_c(ind, a, b, d, p, r, nb, det, sz, s);
  return;
}

void adpcm_adapt_l(Word16 ind, Word16 *a, Word16 *b, Word16 *d, Word16 *p, Word16 *r,
                   Word16 *nb, Word16 *det, Word16 *sz, Word16 *s)
{
  d[0] = mult(*det, qtab4[shr(ind,2)]); move16();
  *nb = logscl (ind, *nb); move16();
  *det = scalel (*nb); move16();
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

Word16 
lsbdec (ilr, mode, s)
Word16 ilr;
Word16 mode;
g722_state *s;
{
  Word16          dl, rl, yl;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  dl = mult(DETL, invqbl_tab[mode][shr(ilr, (Word16)invqbl_shift[mode])]);

  rl = add (SL, dl);              /* recons */
  yl = limit (rl);

  adpcm_adapt_l(ilr, AL, BL, DLT, PLT, RLT, &NBL, &DETL, &SZL, &SL);


  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
quantl5b (Word16 el, Word16 detl)
{
  Word16          sil, mil, val, wd; 


#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((4) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  sil = shr (el, 15);
  wd = sub (MAX_16, s_and(el, MAX_16));
  if(sil == 0)
  {
    wd = el;
    move16();
  }

  val = mult (6288, detl); /*q5b[7]*/
  mil = 3;
  move16();
  if (sub(wd,val) >= 0)
  {
    mil = 11;
    move16();
  }

  val = mult (q5b[mil], detl);
  mil = sub(mil,2);
  if (sub(wd,val) >= 0)
  {
    mil = add(mil,4);
  }

  val = mult (q5b[mil], detl);
  mil = sub(mil,1);
  if (sub(wd,val) >= 0)
  {
    mil = add(mil,2);
  }

  val = mult (q5b[mil], detl);
  if (sub(wd,val) >= 0)
  {
    mil = add(mil,1);
  }
  mil = s_min(mil,14);

  if(sil == 0)
  {
    mil = add(mil, 15);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
quanth (eh, deth)
Word16 eh;
Word16 deth;
{
  Word16          sih, mih, wd;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  sih = shr (eh, 15);
  wd = sub (MAX_16, s_and(eh, MAX_16));
  if(sih == 0)
  {
    wd = eh;
    move16();
  }
  mih = 1;
  move16();
  if (sub(wd, mult (q2, deth)) >= 0)
  {
    mih = 2;
    move16();
  }

  sih = add(sih, 1);
  move16(); /*for the 2 dimensional array addressing */

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
filtep (rlt, al)
Word16 rlt [];
Word16 al [];
{

  Word16          wd1, wd2, spl;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* shift of rlt */
  rlt[2] = rlt[1];  
  rlt[1] = rlt[0];  
  move16();
  move16();

  wd1 = add (rlt[1], rlt[1]);
  wd1 = mult (al[1], wd1);
  wd2 = add (rlt[2], rlt[2]);
  wd2 = mult (al[2], wd2);
  spl = add (wd1, wd2);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
filtez (dlt, bl)
Word16 dlt [];
Word16 bl [];
{
  Word16          szl, wd;
  Word16          i;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((3) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  wd = add (dlt[6], dlt[6]);
  szl = mult (wd, bl[6]);
  FOR (i = 5; i > 0; i--)
  {
    wd = add (dlt[i], dlt[i]);
    wd = mult (wd, bl[i]);
    szl = add (szl, wd);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
limit (rl)
Word16 rl;
{
  rl = bound(rl, -16384, 16383);

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
Word16 
logsch (ih, nbh)
Word16 ih;
Word16 nbh;
{
  Word16          nbph;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  nbph = add ( mult (nbh, 32512), whi[ih]);

  if(nbph < 0)
  {
    nbph = 0;
    move16();
  }
  if(sub(nbph, 22528) > 0)
  {
    nbph = 22528;
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
logscl (il,nbl )
Word16 il;
Word16 nbl;
{
  Word16          ril, nbpl;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  ril = shr (il, 2);
  nbpl = add ( mult (nbl, 32512), wli[ril]);

  if(nbpl < 0)
  {
    nbpl = 0;
    move16();
  }
  if(sub(nbpl, 18432) > 0)
  {
    nbpl = 18432;
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
scalel (nbpl)
Word16 nbpl;
{
  Word16          wd1, wd2;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((2) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  wd1 = s_and(shr(nbpl, 6), 511);
  wd2 = add(wd1, 64);
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
Word16 
scaleh (nbph)
Word16 nbph;
{
  Word16          wd;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  wd = s_and(shr(nbph, 6), 511);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
void 
uppol1 (al, plt)
Word16 al [];
Word16 plt [];
{
  Word16          sg0, sg1, wd1, wd2, wd3, apl1;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((6) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  sg0 = shr (plt[0], 15);
  sg1 = shr (plt[1], 15);
  wd1 = -192;
  move16();
  if(sub(sg0, sg1) == 0)
  {
    wd1 = 192;
    move16();
  }
  wd2 = mult (al[1], 32640);
  apl1 = add (wd1, wd2);
  wd3 = sub (15360, al[2]);
  IF(sub(apl1, wd3) > 0)
  {
    apl1 = wd3;
    move16();
  }
  ELSE
  {
    if(add(apl1, wd3) < 0)
    {
      apl1 = negate (wd3);
    }
  }
  /*  apl1 = (apl1 > wd3) ? wd3 : ((apl1 < -wd3) ? negate (wd3) : apl1);*/

  /* Shift of the plt signals */
  plt[2] = plt[1];
  plt[1] = plt[0];
  al[1] = apl1;
  move16();
  move16();
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
void 
uppol2 (al, plt)
Word16 al [];
Word16 plt [];
{
  Word16  sg0, sg1, sg2, wd1, wd2, wd3, wd4, wd5, apl2;
#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((9) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif

  sg0 = shr (plt[0], 15);
  sg1 = shr (plt[1], 15);
  sg2 = shr (plt[2], 15);
  wd1 = shl (al[1], 2);
  wd2 = add (0, wd1);
  if(sub(sg0, sg1) == 0)
  {
    wd2 = sub (0, wd1);
  }
  wd2 = shr (wd2, 7);
  wd3 = -128;
  move16();
  if(sub(sg0, sg2) == 0)
  {
    wd3 = 128;
    move16();
  }
  wd4 = add (wd2, wd3);
  wd5 = mult (al[2], 32512);
  apl2 = add (wd4, wd5);
  al[2] = bound(apl2, -12288, 12288);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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
void 
upzero (dlt, bl)
Word16 dlt [];
Word16 bl [];
{

  Word16          sg0, sgi, wd1, wd2, wd3;
  Word16          i;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((0) * SIZE_Ptr);
    ssize += (UWord32) ((6) * SIZE_Word16);
    ssize += (UWord32) ((0) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* shift of the dlt line signal and update of bl */

  wd1 = 128;
  move16();
  if(dlt[0] == 0)
  {
    wd1 = 0;
    move16();
  }
  sg0 = shr (dlt[0], 15);

  FOR (i = 6; i > 0; i--)
  {
    sgi = shr (dlt[i], 15);
    wd3 = mult (bl[i], 32640);
    wd2 = sub (wd3, wd1);
    if(sub(sg0, sgi)== 0)
    {
      wd2 = add (wd3, wd1);
    }
    bl[i] = wd2;

    dlt[i] = dlt[i - 1];
    move16();
    move16();
  }
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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

void qmf_tx_buf (Word16 **xin, Word16 *xl, Word16 *xh, Word16 **delayx)
{

  /* Local variables */
  Word16          i;
  Word32          accuma, accumb;
  Word32          comp_low, comp_high;
  Word16          *pcoef, *pdelayx;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((4) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* Saving past samples in delay line */
  *--(*delayx) = *(*xin)++;
  *--(*delayx) = *(*xin)++;
  move16();
  move16();

  /* QMF filtering */
  pcoef = (Word16 *)coef_qmf;

  pdelayx = *delayx;


  accuma = L_mult0(*pcoef++, *pdelayx++);
  accumb = L_mult0(*pcoef++, *pdelayx++);
  FOR (i = 1; i < 12; i++)
  {
    accuma = L_mac0(accuma, *pcoef++, *pdelayx++);
    accumb = L_mac0(accumb, *pcoef++, *pdelayx++);
  }


  comp_low = L_add (accuma, accumb);
  comp_low = L_add (comp_low, comp_low);
  comp_high = L_sub (accuma, accumb);
  comp_high = L_add (comp_high, comp_high);
  *xl = limit ((Word16) L_shr (comp_low, (Word16) 16));
  *xh = limit ((Word16) L_shr (comp_high, (Word16) 16));
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
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

void  qmf_rx_buf (Word16 rl, Word16 rh, Word16 **delayx, Word16 **out)
{
  Word16          i;
  Word32          accuma, accumb;
  Word32          comp_low, comp_high;
  Word16          *pcoef, *pdelayx;

#ifdef DYN_RAM_CNT
  {
    UWord32 ssize = 0;
    ssize += (UWord32) ((2) * SIZE_Ptr);
    ssize += (UWord32) ((1) * SIZE_Word16);
    ssize += (UWord32) ((4) * SIZE_Word32);

    DYN_RAM_PUSH(ssize, "dummy");
  }
#endif
  /* compute sum and difference from lower-band (rl) and higher-band (rh) signals */
  /* update delay line */
  *--(*delayx) = add (rl, rh);
  *--(*delayx) = sub (rl, rh);
  move16();
  move16();

  /* qmf_rx filtering */
  pcoef = (Word16 *)coef_qmf;

  pdelayx = *delayx;

  accuma = L_mult0(*pcoef++, *pdelayx++);
  accumb = L_mult0(*pcoef++, *pdelayx++);
  FOR (i = 1; i < 12; i++)
  {
    accuma = L_mac0(accuma, *pcoef++, *pdelayx++);
    accumb = L_mac0(accumb, *pcoef++, *pdelayx++);
  }

  comp_low = L_shl(accuma,4);
  comp_high = L_shl(accumb,4);

  /* compute output samples */
  *(*out)++ = extract_h(comp_low);
  *(*out)++ = extract_h(comp_high);
  move16();
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
}


/* ******************** End of funcg722.c ***************************** */
