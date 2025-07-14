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

static Word16 Re8_identify_absolute_leader(Word16 y[]);
static void Re8_coord(Word16 *y, Word16 *k);



/*-------------------------------------------------------------------------
* RE8_k2y:
*
* VORONOI INDEXING (INDEX DECODING) k -> y
-------------------------------------------------------------------------*/
void RE8_k2y(
             Word16 *k,    /* i  : Voronoi index k[0..7]                    */
             Word16 m,     /* i  : Voronoi modulo (m = 2^r = 1<<r, where r is integer >=2) */
             Word16 *y     /* o  : 8-dimensional point y[0..7] in RE8    */
             )
{
  Word16 i, v[8], *ptr1, *ptr2, m_tmp, mm;
  Word32 ytp[8], z[8], Ltmp, Lsum ;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((11 * SIZE_Word16) + (18 * SIZE_Word32) + (2 * SIZE_Ptr)), "dummy");
#endif
  /*****************************/

  /*---------------------------------------------------------------*
  * compute y = k M and z=(y-a)/m, where
  *   M = [4        ]
  *       [2 2      ]
  *       [|   \    ]
  *       [2     2  ]
  *       [1 1 _ 1 1]
  *   a=(2,0,...,0)
  *---------------------------------------------------------------*/
  m_tmp = sub(15, m);

  Lsum = L_deposit_l(k[7]);
  ytp[7] = Lsum; move32();
  z[7] = L_shl(Lsum, m_tmp); move32();    /* (int)(floor(y[7]*QR+0.5))>>m */

  FOR (i=6; i>=1; i--)
  {
    Ltmp = L_deposit_l( shl(k[i],1) );
    Lsum = L_add (Lsum, Ltmp);
    ytp[i] = L_add(ytp[7], Ltmp);       move32();
    z[i] = L_shl(ytp[i], m_tmp);        move32();   /* (int)(floor(y[7]*QR+0.5))>>m */

  }

  Lsum = L_add( Lsum, L_deposit_l(shl(k[0],2) ));
  ytp[0] = Lsum; move32();
  z[0] = L_shl(L_sub(Lsum, 2), m_tmp);    move32();   /* (int)(floor(y[7]*QR+0.5))>>m */

  /*---------------------------------------------------------------*
  * find nearest neighbor v of z in infinite RE8
  *---------------------------------------------------------------*/
  RE8_PPV(z, v);

  /*---------------------------------------------------------------*
  * compute y -= m v
  *---------------------------------------------------------------*/
  ptr1=y;                       
  ptr2=v;                       

  mm = shr(shl(1, m), 1); /* shr to remove effect of L_mult in L_msu */

  FOR (i=0; i<8; i++)
  {
    Ltmp = L_msu(ytp[i], *ptr2++, mm);
    *ptr1++ = extract_l(Ltmp);
    move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*----------------------------------------------------------------*
* RE8_vor:
*
* MULTI-RATE RE8 INDEXING BY VORONOI EXTENSION
*----------------------------------------------------------------*/
void RE8_Vor(
             Word16 y[],     /* i  : point in RE8 (8-dimensional integer vector)      */
             Word16 *n,      /* o  : codebook number n=0,2,3,4,... (scalar integer)   */
             Word16 k[],     /* o  : Voronoi index (integer vector of dimension 8) used only if n>4 */
             Word16 c[],     /* o  : codevector in Q0, Q2, Q3, or Q4 if n<=4, y=c */
             Word16 *ka      /* o  : identifier of absolute leader (needed to index c)*/
             )
{
  Word16 i, r, iter, ka_tmp, n_tmp, mask;
  Word16 k_tmp[8], v[8], c_tmp[8], k_mod[8];
  Word32 Ltmp, Lsphere;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((38 * SIZE_Word16) + (2 * SIZE_Word32)), "dummy");
#endif
  /*****************************/

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
  *ka = Re8_identify_absolute_leader(y); move16();

  /*----------------------------------------------------------------*
  * compute codebook number n of Qn (by table look-up)
  *   at this stage, n=0,2,3,4 or out=100
  *----------------------------------------------------------------*/
  *n = Da_nq_[*ka];                      move16();

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

  IF (sub(*n, 4) <= 0)
  {
    mov16( 8, y, c);
  }
  ELSE
  {
    /*------------------------------------------------------------*
    * initialize r and m=2^r based on || y ||^2/8
    *------------------------------------------------------------*/
    Ltmp = L_mac_Array(8, y, y);

    Lsphere = L_shr(Ltmp, 5+1); /* *0.125*0.25  / 2 to remove L_mac effect */

    r = 1;                 move16();
    WHILE (L_sub(Lsphere, 11) > 0)
    {
      r = add(r, 1);
      Lsphere = L_shr(Lsphere, 2); /* *= 0.25 */
    }
    /*------------------------------------------------------------*
    * compute the coordinates of y in the RE8 basis
    *------------------------------------------------------------*/
    Re8_coord(y, k_mod);

    /*------------------------------------------------------------*
    * compute m and the mask needed for modulo m (for Voronoi coding)
    *------------------------------------------------------------*/
    mask = sub(shl(1, r), 1); /* 0x0..011...1 */
    /*------------------------------------------------------------*
    * find the minimal value of r (or equivalently of m) in 2 iterations
    *------------------------------------------------------------*/

    FOR (iter=0; iter<2; iter++)
    {
      /*--------------------------------------------------------*
      * compute v such that y is in m RE_8 +v (by Voronoi coding)
      *--------------------------------------------------------*/
      FOR (i=0; i<8; i++)
      {
        k_tmp[i] = s_and( k_mod[i], mask); move16();

      }

      RE8_k2y(k_tmp, r, v);

      /*--------------------------------------------------------*
      * compute c = (y-v)/m
      * (y is in RE8, c is also in RE8 by definition of v)
      *--------------------------------------------------------*/

      FOR (i=0; i<8; i++)
      {
        c_tmp[i] = shr_r(sub(y[i], v[i]), r);  move16();
      }

      /*--------------------------------------------------------*
      *  verify if c_tmp is in Q2, Q3 or Q4
      *--------------------------------------------------------*/
      ka_tmp = Re8_identify_absolute_leader(c_tmp);

      /*--------------------------------------------------------*
      * at this stage, n_tmp=2,3,4 or out = 100 -- n=0 is not possible
      *--------------------------------------------------------*/
      n_tmp = Da_nq_[ka_tmp];                    move16();

      IF (sub(n_tmp, 4) > 0)
      {
        /*--------------------------------------------------------*
        * if c is not in Q2, Q3, or Q4 (i.e. n_tmp>4), use m = 2^(r+1) instead of 2^r
        *--------------------------------------------------------*/
        r = add(r, 1);
        mask = add(shl(mask, 1), 1); /* mask = m-1 <- this is less complex */
      }
      ELSE
      {
        /*--------------------------------------------------------*
        * c is in Q2, Q3, or Q4 -> the decomposition of y as y = m c + v is valid
        *
        * since Q2 is a subset of Q3, indicate n=3 instead of n=2 (this is because
        * for n>4, n=n'+2r with n'=3 or 4, so n'=2 is not valid)
        *--------------------------------------------------------*/
        n_tmp = s_max(n_tmp, 3);

        /*--------------------------------------------------------*
        * save current values into ka, n, k and c
        *--------------------------------------------------------*/
        *ka = ka_tmp;                          move16();
        *n = add(n_tmp, shl(r, 1));            move16();
        mov16( 8, k_tmp, k);
        mov16( 8, c_tmp, c);
        /*--------------------------------------------------------*
        * try  m = 2^(r-1) instead of 2^r to be sure that m is minimal
        *--------------------------------------------------------*/
        r = sub(r, 1);
        mask = shr(mask, 1);
      }
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*-----------------------------------------------------------------------*
* re8_identify_absolute_leader:
*
* IDENTIFY THE ABSOLUTE LEADER RELATED TO y USING A PRE-DEFINED TABLE WHICH
* SPECIFIES THE CODEBOOKS Q0, Q2, Q3 and Q4
-----------------------------------------------------------------------*/
static Word16 Re8_identify_absolute_leader(  /* o : integer indicating if y if in Q0, Q2, */
                                           /*     Q3 or Q4 (or if y is an outlier)      */
                                           Word16 y[]                         /* i : point in RE8 (8-dimensional integer vector) */
)
{
  Word16 i,s,id,nb,pos,ka, tmp16;
  Word32 Ltmp, Ls;
  Word32 C;
  Word16 *ptr;


  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((7 * SIZE_Word16) + (3 * SIZE_Word32) + SIZE_Ptr), "dummy");
#endif
  /*****************************/

  /*-----------------------------------------------------------------------*
  * compute the RE8 shell number s = (y1^2+...+y8^2)/8 and C=(y1^2, ..., y8^2)
  *-----------------------------------------------------------------------*/
  Ls = L_mac_Array(8, y, y);
  s = extract_h_L_shl(Ls, 16-(3+1)); /* s can saturate here */

  /*-----------------------------------------------------------------------*
  * compute the index 0 <= ka <= NB_LEADER+1 which identifies an absolute leader of Q0, Q2, Q3 or Q4
  *
  * by default, ka=index of last element of the table (to indicate an outlier)
  *-----------------------------------------------------------------------*/
  /*-------------------------------------------------------------------*
  * if s=0, y=0 i.e. y is in Q0 -> ka=index of element indicating Q0
  *-------------------------------------------------------------------*/
  ka = NB_LEADER;               move16();
  IF (s != 0)
  {
    ka = NB_LEADER+1;             move16();
    /*-------------------------------------------------------------------*
    * the maximal value of s for y in  Q0, Q2, Q3 or Q4 is NB_SPHERE
    *   if s> NB_SPHERE, y is an outlier (the value of ka is set correctly)
    *-------------------------------------------------------------------*/
    IF (sub(s, NB_SPHERE) <= 0)
    {
      /*---------------------------------------------------------------*
      * compute the unique identifier id of the absolute leader related to y:
      * s = (y1^4 + ... + y8^4)/8
      *---------------------------------------------------------------*/
      C = L_mult(y[0], y[0]);
      tmp16 = extract_h_L_shl(C, 16-1);
      Ltmp = L_mult(tmp16, tmp16);
      FOR (i=1; i<8; i++)
      {
        C = L_mult(y[i], y[i]);
        tmp16 = extract_h_L_shl(C, 16-1);
        Ltmp = L_mac(Ltmp, tmp16, tmp16);
      }
      id = extract_h_L_shl(Ltmp, 16-(3+1)); /* id can saturate to 8192 */

      /*---------------------------------------------------------------*
      * search for id in table Da_id
      * (containing all possible values of id if y is in Q2, Q3 or Q4)
      * this search is focused based on the shell number s so that
      * only the id's related to the shell of number s are checked
      *---------------------------------------------------------------*/

      nb = Da_nb_[s - 1]; /* get the number of absolute leaders used on the shell of number s */
      pos = Da_pos_[s - 1]; /* get the position of the first absolute leader of shell s in Da_id */
      move16(); move16();

      ptr = &Da_id_[pos]; move16();
      FOR (i=0; i<nb; i++)
      {
        IF (sub(id, *ptr) == 0)
        {
          ka = pos;         move16(); /* get ka */
          BREAK;
        }
        ptr++;
        pos = add(pos,1);
      }
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return(ka);
}

/*-------------------------------------------------------------------------
* Re8_coord:
*
* COMPUTATION OF RE8 COORDINATES
-----------------------------------------------------------------------*/
static void Re8_coord(
                      Word16 *y,    /* i  : 8-dimensional point y[0..7] in RE8 */
                      Word16 *k     /* o  : coordinates k[0..7] */
                      )
{
  Word16 i, tmp, sum;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (3 * SIZE_Word16), "dummy");
#endif
  /*****************************/

  /*---------------------------------------------------------------*
  * compute k = y M^-1
  *   M = 1/4 [ 1          ]
  *           [-1  2       ]
  *           [ |    \     ]
  *           [-1       2  ]
  *           [ 5 -2 _ -2 4]
  *
  *---------------------------------------------------------------*/
  k[7] = y[7];                       move16();
  tmp = y[7];                        move16();
  sum = add(y[7], shl(y[7], 2));

  FOR(i=6; i>=1; i--)
  {
    /* apply factor 2/4 from M^-1 */
    k[i] = shr(sub(y[i], tmp), 1); move16();
    sum  = sub(sum, y[i]);
  }
  /* apply factor 1/4 from M^-1 */
  k[0]= shr(add(y[0], sum), 2);      move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*--------------------------------------------------------------*
* Sort:
*
* SORT SUBVECTORS BY DECREASING BIT ALLOCATIONS
*--------------------------------------------------------------*/
void Sort(
          Word16 *ebits,    /* i  : estimated bit allocations (table of n *positive* integers) */
          Word16 n,         /* i  : number of subvectors        */
          Word16 *idx,      /* o  : indices                     */
          Word16 *t         /* o  : temporary buffer            */
          )
{
  Word16 i, j, ebits_max, pos;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Word16), "dummy");
#endif
  /*****************************/

  mov16(n, ebits, t);


  FOR (i=0; i<n; i++)
  {
    ebits_max = t[0];                              move16();
    pos = 0;                                       move16();
    FOR (j=1; j<n; j++)
    {
      if (sub(t[j], ebits_max) > 0)
      {
        pos = j;                               move16();
      }
      ebits_max = s_max(t[j], ebits_max);
    }
    idx[i] = pos;                                  move16();
    t[pos] = -1;                                   move16();
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}
