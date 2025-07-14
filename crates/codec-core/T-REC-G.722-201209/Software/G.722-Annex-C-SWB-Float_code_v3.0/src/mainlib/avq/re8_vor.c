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
static void re8_coord(Short *y, Short *k);
static Short re8_identify_absolute_leader(Short y[]);



/*-------------------------------------------------------------------------
* RE8_k2y:
*
* VORONOI INDEXING (INDEX DECODING) k -> y
-------------------------------------------------------------------------*/
void RE8_k2y_flt(
             Short *k,    /* i  : Voronoi index k[0..7]                    */
             Short m,     /* i  : Voronoi modulo (m = 2^r = 1<<r, where r is integer >=2) */
             Short *y     /* o  : 8-dimensional point y[0..7] in RE8    */
)
{
  int i, tmp, sum;
  Short v[8], *ptr1, *ptr2;
  Float z[8], mm;

  mm = (Float)(1 << m);

  /* compute y = k M and z=(y-a)/m, where
  M = [4        ]
  [2 2      ]
  [|   \    ]
  [2     2  ]
  [1 1 _ 1 1]
  a=(2,0,...,0)
  */
  for (i=0; i<8; i++)
  {
    y[i] = k[7];
  }
  z[7] = (float)y[7]/mm;

  sum=0;

  for (i=6; i>=1; i--)
  {
    tmp   = 2*k[i];
    sum  += tmp;
    y[i] += tmp;
    z[i] = (float)y[i]/mm;
  }
  y[0] += (4*k[0] + sum);

  z[0] = (float)(y[0]-2)/mm;

  /* find nearest neighbor v of z in infinite RE8 */
  RE8_ppv(z,v);

  /* compute y -= m v */
  ptr1=y;
  ptr2=v;
  for (i=0; i<8; i++)
  {
    *ptr1 = (Short)(*ptr1 - mm * *ptr2++);
    ptr1++;
  }
}



/*----------------------------------------------------------------*
* RE8_vor:
*
* MULTI-RATE RE8 INDEXING BY VORONOI EXTENSION
*----------------------------------------------------------------*/
void RE8_vor(
             Short y[],     /* i  : point in RE8 (8-dimensional integer vector)      */
             Short *n,      /* o  : codebook number n=0,2,3,4,... (scalar integer)   */
             Short k[],     /* o  : Voronoi index (integer vector of dimension 8) used only if n>4 */
             Short c[],     /* o  : codevector in Q0, Q2, Q3, or Q4 if n<=4, y=c */
             Short *ka      /* o  : identifier of absolute leader (needed to index c)*/
             )
{
  Short i, r, iter, ka_tmp, n_tmp, mask;
  Short k_tmp[8], v[8], c_tmp[8], k_mod[8];
  long Ltmp, Lsphere;

  /*----------------------------------------------------------------*
  * verify if y is in Q0, Q2, Q3 or Q4
  *   (a fast search is used here:
  *    the codebooks Q0, Q2, Q3 or Q4 are specified in terms of RE8 absolute leaders
  *    (see FORinstance Xie and Adoul's paper in ICASSP 96)
  *    - a unique code identifying the absolute leader related to y is computed
  *      in re8_identify_absolute_leader()
  *      this code is searched FORin a pre-defined list which specifies Q0, Q2, Q3 or Q4)
  *      the absolute leader is identified by ka
  *    - a translation table maps ka to the codebook number n)
  *----------------------------------------------------------------*/
  *ka = re8_identify_absolute_leader(y);

  /*----------------------------------------------------------------*
  * compute codebook number n of Qn (by table look-up)
  *   at this stage, n=0,2,3,4 or out=100
  *----------------------------------------------------------------*/
  *n = Da_nq_[*ka];

  /*----------------------------------------------------------------*
  * decompose y into :
  *     (if n<=4:)
  *     y = c        where c is in Q0, Q2, Q3 or Q4
  *   or
  *     (if n>4:)
  *     y = m c + v  where c is in Q3 or Q4, v is a Voronoi codevector
  *                        m=2^r (r integer >=2)
  *
  *   in the latter case (if n>4), as a side-product, compute the (Voronoi) index k[] of v
  *   and replace n by n = n' + 2r where n' = 3 or 4 (c is in Qn') and r is defined above
  *----------------------------------------------------------------*/

  if (*n <= 4)
  {
    movSS(8, y, c);
  }
  else
  {
    /*------------------------------------------------------------*
    * initialize r and m=2^r based on || y ||^2/8
    *------------------------------------------------------------*/
    Ltmp = 0;
    for (i=0; i<8; i++)
    {
      Ltmp = Ltmp + y[i] * y[i];
    }

    Lsphere = Ltmp >> 5; /* *0.125*0.25 */

    r = 1;
    while (Lsphere > 11)
    {
      r = r + 1;
      Lsphere = Lsphere >> 2; /* *= 0.25 */
    }
    /*------------------------------------------------------------*
    * compute the coordinates of y in the RE8 basis
    *------------------------------------------------------------*/
    re8_coord(y, k_mod);

    /*------------------------------------------------------------*
    * compute m and the mask needed for modulo m (for Voronoi coding)
    *------------------------------------------------------------*/
    mask = (1<<r) - 1; /* 0x0..011...1 */
    /*------------------------------------------------------------*
    * find the minimal value of r (or equivalently of m) in 2 iterations
    *------------------------------------------------------------*/

    for (iter=0; iter<2; iter++)
    {
      /*--------------------------------------------------------*
      * compute v such that y is in m RE_8 +v (by Voronoi coding)
      *--------------------------------------------------------*/
      for (i=0; i<8; i++)
      {
        k_tmp[i] = k_mod[i] & mask;
      }

      RE8_k2y_flt(k_tmp, r, v);

      /*--------------------------------------------------------*
      * compute c = (y-v)/m
      * (y is in RE8, c is also in RE8 by definition of v)
      *--------------------------------------------------------*/
      n_tmp = 1 << (r - 1); // for Rounding
      for (i=0; i<8; i++)
      {
        c_tmp[i] = (y[i] - v[i] + n_tmp) >> r;
      }

      /*--------------------------------------------------------*
      *  verify if c_tmp is in Q2, Q3 or Q4
      *--------------------------------------------------------*/
      ka_tmp = re8_identify_absolute_leader(c_tmp);

      /*--------------------------------------------------------*
      * at this stage, n_tmp=2,3,4 or out = 100 -- n=0 is not possible
      *--------------------------------------------------------*/
      n_tmp = Da_nq_[ka_tmp];

      if (n_tmp > 4)
      {
        /*--------------------------------------------------------*
        * if c is not in Q2, Q3, or Q4 (i.e. n_tmp>4), use m = 2^(r+1) instead of 2^r
        *--------------------------------------------------------*/
        r = r + 1;
        mask = (mask << 1) + 1; /* mask = m-1 <- this is less complex */
      }
      else
      {
        /*--------------------------------------------------------*
        * c is in Q2, Q3, or Q4 -> the decomposition of y as y = m c + v is valid
        *
        * since Q2 is a subset of Q3, indicate n=3 instead of n=2 (this is because
        * for n>4, n=n'+2r with n'=3 or 4, so n'=2 is not valid)
        *--------------------------------------------------------*/
        if (n_tmp < 3)
            n_tmp = 3;

        /*--------------------------------------------------------*
        * save current values into ka, n, k and c
        *--------------------------------------------------------*/
        *ka = ka_tmp;
        *n = n_tmp + (r<<1);
        movSS( 8, k_tmp, k);
        movSS( 8, c_tmp, c);
        /*--------------------------------------------------------*
        * try  m = 2^(r-1) instead of 2^r to be sure that m is minimal
        *--------------------------------------------------------*/
        r = r - 1;
        mask = mask >> 1;
      }
    }
  }
}



/*-----------------------------------------------------------------------*
* re8_identify_absolute_leader:
*
* IDENTIFY THE ABSOLUTE LEADER RELATED TO y USING A PRE-DEFINED TABLE WHICH
* SPECIFIES THE CODEBOOKS Q0, Q2, Q3 and Q4
-----------------------------------------------------------------------*/
static Short re8_identify_absolute_leader(/* o : integer indicating if y if in Q0, Q2, */
                                          /*     Q3 or Q4 (or if y is an outlier)      */
                                          Short y[]/* i : point in RE8 (8-dimensional integer vector) */
)
{
    Short i, id, nb, pos, ka;
    long s, C[8], tmp;

   /*-----------------------------------------------------------------------*
    * compute the RE8 shell number s = (y1^2+...+y8^2)/8 and C=(y1^2, ..., y8^2)
    *-----------------------------------------------------------------------*/
    s=0;
    for (i=0;i<8;i++)
    {
      C[i] = (long)y[i]*y[i];

      s += C[i];
    }
    s >>= 3;

   /*-----------------------------------------------------------------------*
    * compute the index 0 <= ka <= NB_LEADER+1 which identifies an absolute leader of Q0, Q2, Q3 or Q4
    *
    * by default, ka=index of last element of the table (to indicate an outlier)
    *-----------------------------------------------------------------------*/
    ka = NB_LEADER+1; /* by default, ka=index of last element of the table (to indicate an outlier) */
    if (s == 0)
    {
     /*-------------------------------------------------------------------*
      * if s=0, y=0 i.e. y is in Q0 -> ka=index of element indicating Q0
      *-------------------------------------------------------------------*/
      ka = NB_LEADER;
    }
    else
    {
     /*-------------------------------------------------------------------*
      * the maximal value of s for y in  Q0, Q2, Q3 or Q4 is NB_SPHERE
      *   if s> NB_SPHERE, y is an outlier (the value of ka is set correctly)
      *-------------------------------------------------------------------*/
      if (s <= NB_SPHERE)
      {
       /*---------------------------------------------------------------*
        * compute the unique identifier id of the absolute leader related to y:
        * s = (y1^4 + ... + y8^4)/8
        *---------------------------------------------------------------*/
        tmp=0;
        for (i=0;i<8;i++)
        {
          tmp += C[i]*C[i];
        }
        id = (Short)(tmp >> 3);
       /*---------------------------------------------------------------*
        * search for id in table Da_id
        * (containing all possible values of id if y is in Q2, Q3 or Q4)
        * this search is focused based on the shell number s so that
        * only the id's related to the shell of number s are checked
        *---------------------------------------------------------------*/
        nb = Da_nb_[s-1]; /* get the number of absolute leaders used on the shell of number s */
        pos = Da_pos_[s-1]; /* get the position of the first absolute leader of shell s in Da_id */
        for (i=0; i<nb; i++)
        {

          if (id == Da_id_[pos])
          {
            ka = pos; /* get ka */
            break;
          }
          pos++;
        }
      }
    }

    return(ka);
}



/*-------------------------------------------------------------------------
* re8_coord:
*
* COMPUTATION OF RE8 COORDINATES
-----------------------------------------------------------------------*/
static void re8_coord(
                      Short *y,    /* i  : 8-dimensional point y[0..7] in RE8 */
                      Short *k     /* o  : coordinates k[0..7] */
                      )
{
  Short i, tmp, sum;

  /*---------------------------------------------------------------*
  * compute k = y M^-1
  *   M = 1/4 [ 1          ]
  *           [-1  2       ]
  *           [ |    \     ]
  *           [-1       2  ]
  *           [ 5 -2 _ -2 4]
  *
  *---------------------------------------------------------------*/
  k[7] = y[7];
  tmp = y[7]; 
  sum = 5*y[7];

  for (i=6; i>=1; i--)
  {
    /* apply factor 2/4 from M^-1 */
    k[i] = k[i] = (y[i]-tmp) >> 1;
    sum -= y[i];
  }
  /* apply factor 1/4 from M^-1 */
  k[0]= k[0] = (y[0]+sum) >> 2;
}



/*--------------------------------------------------------------*
* sort:
*
* SORT SUBVECTORS BY DECREASING BIT ALLOCATIONS
*--------------------------------------------------------------*/
void sort(
          Short *ebits,    /* i  : estimated bit allocations (table of n *positive* integers) */
          Short n,         /* i  : number of subvectors        */
          Short *idx,      /* o  : indices                     */
          Short *t         /* o  : temporary buffer            */
          )
{
  Short i, j, ebits_max, pos;

  movSS(n, ebits, t);

  for (i=0; i<n; i++)
  {
    ebits_max = t[0];
    pos = 0;
    for (j=1; j<n; j++)
    {
      if (t[j] > ebits_max)
      {
        pos = j;
        ebits_max = t[j];
      }
    }
    idx[i] = pos;                           
    t[pos] = -1;                            
  }
}


/*--------------------------------------------------------------*
* Sort:
*
* SORT SUBVECTORS BY DECREASING BIT ALLOCATIONS
*--------------------------------------------------------------*/
void f_Sort(
          Float *ebits,    /* i  : estimated bit allocations (table of n *positive* integers) */
          Short n,         /* i  : number of subvectors        */
          Short *idx,      /* o  : indices                     */
          Float *t         /* o  : temporary buffer            */
          )
{
  Short i, j, pos;
  Float ebits_max;

  movF(n, ebits, t);


  for(i=0; i<n; i++)
  {
    ebits_max = t[0];
    pos = 0;
    for(j=1; j<n; j++)
    {
      if( t[j] > ebits_max )
      {
        pos = j;
      }
      ebits_max = f_max(t[j], ebits_max);
    }
    idx[i] = pos;
    t[pos] = -1.0f;
  }

}
