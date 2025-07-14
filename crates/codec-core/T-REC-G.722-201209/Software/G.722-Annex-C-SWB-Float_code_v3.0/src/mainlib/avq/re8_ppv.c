/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include "rom.h"
#include "re8.h"


/*-------------------------------------------------------------------*
* Prototypes
*-------------------------------------------------------------------*/
static void nearest_neighbor_2D8(float x[], Short y[]);
static Float compute_error_2D8(const Float x[], const Short y[]);



/*--------------------------------------------------------------*
* RE8_ppv:
*
* NEAREST NEIGHBOR SEARCH IN INFINITE LATTICE RE8
* the algorithm is based on the definition of RE8 as
*     RE8 = (2D8) U (2D8+[1,1,1,1,1,1,1,1])
* it applies the coset decoding of Sloane and Conway
* --------------------------------------------------------------*/
void RE8_ppv(
             Float x[],  /* i  : point in R^8Q15*/
             Short y[]   /* o  : point in RE8 (8-dimensional integer vector) */
)
{
  Short i,y0[8];
  Float e0,e1,x1[8];

  /*--------------------------------------------------------------*
  * find the nearest neighbor y0 of x in 2D8
  *--------------------------------------------------------------*/
  nearest_neighbor_2D8(x, y0);

  /*--------------------------------------------------------------*
  * find the nearest neighbor y1 of x in 2D8+(1,...,1) (by coset decoding)
  *--------------------------------------------------------------*/
  for (i=0;i<8;i++)
  {
    x1[i]=x[i]-1.0f;
  }

  nearest_neighbor_2D8(x1, y);

  for (i=0;i<8;i++)
  {
    y[i]+=1;
  }

  /*--------------------------------------------------------------*
  * compute e0=||x-y0||^2 and e1=||x-y1||^2
  *--------------------------------------------------------------*/

  e0 = compute_error_2D8(x, y0);
  e1 = compute_error_2D8(x, y);

  /*--------------------------------------------------------------*
  * select best candidate y0 or y1 to minimize distortion
  *--------------------------------------------------------------*/
  if (e0<e1)
  {
    for (i=0;i<8;i++)
    {
      y[i]=y0[i];
    }
  }

  return;
}



/*--------------------------------------------------------------*
* nearest_neighbor_2D8(x,y)
*
* NEAREST NEIGHBOR SEARCH IN INFINITE LATTICE 2D8
* algorithm: nn_2D8(x) = 2*nn_D8(x/2)
*            nn_D8 = decoding of Z^8 with Wagner rule
* (see Conway and Sloane's paper in IT-82)
--------------------------------------------------------------*/
static void nearest_neighbor_2D8(Float x[], Short y[])
{
  Short i,j, sum;
  Float s,e[8],em;

  /*--------------------------------------------------------------*
  * round x into 2Z^8 i.e. compute y=(y1,...,y8) such that yi = 2[xi/2]
  *   where [.] is the nearest integer operator
  *   in the mean time, compute sum = y1+...+y8
  *--------------------------------------------------------------*/
  sum=0;
  for (i=0;i<8;i++)
  {
    /* round to ..., -2, 0, 2, ... ([-1..1[ --> 0) */
    if (x[i] < 0)
    {
      y[i] = -2*(((Short)(1.0-x[i]))>>1);

    }
    else
    {
      y[i] = 2*(((Short)(1.0+x[i]))>>1);

    }
    sum += y[i];
  }

  /*--------------------------------------------------------------*
  * check if y1+...+y8 is a multiple of 4
  *   if not, y is not round xj in the wrong way where j is defined by
  *   j = arg max_i | xi -yi|
  *   (this is called the Wagner rule)
  *--------------------------------------------------------------*/

  if (sum%4)
  {
    /* find j = arg max_i | xi -yi| */
    em=0;
    j=0;
    for (i=0;i<8;i++)
    {
      /* compute ei = xi-yi */
      /* compute |ei| = | xi-yi | */
      e[i]=x[i]-y[i];
      if (e[i]<0)
      {
        s=-e[i];
      }
      else
      {
        s=e[i];
      }
      /* check if |ei| is maximal, if so, set j=i */
      if (em<s)
      {
        em=s;
        j=i;
      }
    }
    /* round xj in the "wrong way" */
    if (e[j]<0)
    {
      y[j]-=2;
    }
    else
    {
      y[j]+=2;
    }
  }

  return;
}



/*--------------------------------------------------------------*
* compute_error_2D8(x,y)
*
* Compute mean square error between input vector and
* (quantized) point in 2D8.
--------------------------------------------------------------*/
static Float compute_error_2D8(    /* o  : mean squared error                          */
                                const Float x[], /* i  : input vector                                */
                                const Short y[]  /* i  : point in 2D8 (8-dimensional integer vector) */
)
{
  Short i;
  Float e, tmp;

  e = 0.0f;
  for (i=0;i<8;i++)
  {
      tmp = x[i]-y[i];
      e += (tmp*tmp);
  }
  return e;
}
