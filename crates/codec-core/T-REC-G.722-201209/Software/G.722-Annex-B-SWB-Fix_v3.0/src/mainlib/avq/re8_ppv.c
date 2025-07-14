/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "rom.h"
#include "re8.h"
#include "dsputil.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------*
* Prototypes
*-------------------------------------------------------------------*/
static void Nearest_neighbor_2D8(Word32 x[], Word16 y[]);
static Word32 Compute_error_2D8(const Word32 x[], const Word16 y[]);

/*--------------------------------------------------------------*
* RE8_PPV:
*
* NEAREST NEIGHBOR SEARCH IN INFINITE LATTICE RE8
* the algorithm is based on the definition of RE8 as
*     RE8 = (2D8) U (2D8+[1,1,1,1,1,1,1,1])
* it applies the coset decoding of Sloane and Conway
* --------------------------------------------------------------*/
void RE8_PPV(
             Word32 x[],     /* i  : point in R^8Q15*/
             Word16 y[]      /* o  : point in RE8 (8-dimensional integer vector) */
)
{
  Word16 i, y0[8];
  Word32 e0, e1, x1[8];

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((9 * SIZE_Word16) + (10 * SIZE_Word32)), "dummy");
#endif
  /*****************************/


  /*--------------------------------------------------------------*
  * find the nearest neighbor y0 of x in 2D8
  *--------------------------------------------------------------*/
  Nearest_neighbor_2D8(x, y0);

  /*--------------------------------------------------------------*
  * find the nearest neighbor y1 of x in 2D8+(1,...,1) (by coset decoding)
  *--------------------------------------------------------------*/


  FOR (i=0; i<8; i++)
  {
    x1[i] = L_sub(x[i], QR);   move32();
  }
  Nearest_neighbor_2D8(x1, y);

  array_oper(8, 1, y, y, &add);

  /*--------------------------------------------------------------*
  * compute e0=||x-y0||^2 and e1=||x-y1||^2
  *--------------------------------------------------------------*/

  e0 = Compute_error_2D8(x, y0);
  e1 = Compute_error_2D8(x, y);


  /*--------------------------------------------------------------*
  * select best candidate y0 or y1 to minimize distortion
  *--------------------------------------------------------------*/
  IF (L_sub(e0, e1) < 0)
  {
    mov16(8, y0, y);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*--------------------------------------------------------------*
* Nearest_neighbor_2D8(x,y)
*
* NEAREST NEIGHBOR SEARCH IN INFINITE LATTICE 2D8
* algorithm: nn_2D8(x) = 2*nn_D8(x/2)
*            nn_D8 = decoding of Z^8 with Wagner rule
* (see Conway and Sloane's paper in IT-82)
--------------------------------------------------------------*/
static void Nearest_neighbor_2D8(
                                 Word32 x[],     /* i  : point in R^8                                */
                                 Word16 y[]      /* o  : point in 2D8 (8-dimensional integer vector) */
)
{
  Word16 i,j;
  Word16 sum, tmp16, tmp16b;
  Word32 s, e, em;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((5 * SIZE_Word16) + (3 * SIZE_Word32)), "dummy");
#endif
  /*****************************/

  /*--------------------------------------------------------------*
  * round x into 2Z^8 i.e. compute y=(y1,...,y8) such that yi = 2[xi/2]
  *   where [.] is the nearest integer operator
  *   in the mean time, compute sum = y1+...+y8
  *--------------------------------------------------------------*/
  sum = 0;                                       move16();

  FOR (i=0; i<8; i++)
  {
    /* round to ..., -2, 0, 2, ... ([-1..1[ --> 0) */
    tmp16 = round_fx(L_add(x[i], L_shr(x[i], 31)));
    y[i] = shl(tmp16, 1);                      move16();
    /* sum += y[i] */
    sum = add(sum, y[i]);
  }

  /*--------------------------------------------------------------*
  * check if y1+...+y8 is a multiple of 4
  *   if not, y is not round xj in the wrong way where j is defined by
  *   j = arg max_i | xi -yi|
  *   (this is called the Wagner rule)
  *--------------------------------------------------------------*/
  IF (s_and(sum, 2) != 0)
  {
    /* find j = arg max_i | xi -yi| */
    em = L_deposit_l(0);
    j = 0;                                     move16();

    FOR (i=0; i<8; i++)
    {
      /* compute ei = xi-yi */
      /* e[i]=x[i]-y[i]     */
      e = L_msu(x[i], y[i], QR/2);

      /* compute |ei| = | xi-yi | */
      s = L_abs(e);

      /* check if |ei| is maximal, if so, set j=i */
      if (L_sub(em, s) < 0)
      {
        j = i;                             move16();
      }
      em = L_max(s, em);
    }

    /* round xj in the "wrong way" */
    e = L_msu(x[j], y[j], QR/2);
    tmp16 = extract_h(e);
    tmp16b = add(y[j], 2);    

    if(tmp16 < 0)
    {
      tmp16b = sub(tmp16b, 2+2);              
    }
    y[j] = tmp16b;             move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
}


/*--------------------------------------------------------------*
* Compute_error_2D8(x,y)
*
* Compute mean square error between input vector and
* (quantized) point in 2D8.
--------------------------------------------------------------*/
static Word32 Compute_error_2D8(    /* o  : mean squared error                          */
                                const Word32 x[],               /* i  : input vector                                */
                                const Word16 y[]                /* i  : point in 2D8 (8-dimensional integer vector) */
)
{
  Word16 i, hi, lo;
  Word32 err, Ltmp;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((3 * SIZE_Word16) + (2 * SIZE_Word32)), "dummy");
#endif
  /*****************************/

  err = L_deposit_l(0);
  FOR (i=0; i<8; i++)
  {
    /*tmp = x[i]-y[i];*/
    Ltmp = L_msu(x[i], y[i], 16384);
    hi   = extract_h_L_shl(Ltmp, 1);
    lo   = extract_l(L_msu(Ltmp, hi, 16384));

    Ltmp = L_mult(hi, hi);
    Ltmp = L_shl(Ltmp, 14);
    Ltmp = L_mac(Ltmp, hi, lo);
    Ltmp = L_mac0(Ltmp, mult(lo, lo), 1);

    /* err+=tmp*tmp */
    err = L_add(Ltmp, err);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return(err);
}
