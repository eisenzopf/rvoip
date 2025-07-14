/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include "re8.h"
#include "avq.h"
#include "bit_op.h"
#include "bwe.h"
#include "log2.h"          /* for Log2_norm_lc */
#include "math_op.h"       /* for Pow2 */
#include "oper_32b.h"      /* for L_Extract_lc */

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------*
* Function prototypes
*-------------------------------------------------------------------*/

static void RE8_Cod(
                    Word16 x[],  /* i  : point in RE8 (8-dimensional integer vector)                         */
                    Word16 *n,   /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})    */
                    UWord16 *I,   /* o  : index of c (pointer to unsigned 16-bit word)                        */
                    Word16 k[]   /* o  : index of v (8-dimensional vector of binary indices) = Voronoi index */
);

/*--------------------------------------------------------------*
* Calc_bits(nq)
*
* COMPUTE (NUMBER OF BITS -1) TO DESCRIBE Q #nq
*--------------------------------------------------------------*/
static Word16 Calc_bits( /* o  : bit allocation            */
                        Word16 nq            /* i  : quantizer id (0,2,3,4...) */
                        )
{
  IF (sub(nq, 2) >= 0)
  {
    /*-----------------------------------------------------*
    * 4n bits + variable-length descriptor for allocation:
    *  descriptor -> nq
    *  0          -> 0
    *  10         -> 2
    *  110        -> 3
    *  => size of descriptor = 5n bits
    *-----------------------------------------------------*/
    return sub(add(shl(nq, 2), nq), 1); /* [5n-1] */
  }
  return 0; /* 1-1 [1 bit to describe the allocation] */
}

/*-----------------------------------------------------------------*
*   Function  AVQ_Encmux_Bstr                                     *
*            ~~~~~~~~~~~~~~~                                      *
*   Encode subvectors and write indexes into one bitstream.       *
*-----------------------------------------------------------------*/

Word16 AVQ_Encmux_Bstr(   /* o:   number of unused bits                        */
                       Word16 xriq[],  /* i/o: rounded subvectors [0..8*Nsv-1] followed
                                       by rounded bit allocations [8*Nsv..8*Nsv+Nsv-1] */
                                       UWord16 **pBst,  /* i/o: pointer to the bitstream buffer              */
                                       const Word16 nb_bits, /* i:   number of allocated bits                     */
                                       const Word16 Nsv      /* i:   number of subvectors                         */
                                       )
{
  Word16 i, j, bits, pos, pos_max, overflow;
  Word16 sort_idx[NSV_MAX];
  Word16 nq[NSV_MAX], kv[NSV_MAX*8];
  UWord16 I[NSV_MAX];
  Word16 tmp16;
  Word16 *ptr;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (((7 + (11 * NSV_MAX)) * SIZE_Word16) + SIZE_Ptr), "dummy");
#endif
  /*****************************/

  /*  encode subvectors and fix possible overflows in TOTAL bit budget:
  i.e. find for each subvector a codebook index nq (nq=0,2,3,4,...),
  a base codebook index (I), and a Voronoi index (kv)                         */
  /* ============================================================================ */

  /* sort subvectors by estimated bit allocations in decreasing order */
  /* use vector 'kv' as temporary to save memory */
  Sort(&xriq[shl(Nsv,3)], Nsv, sort_idx, kv);


  /* compute multi-rate indices and avoid bit budget overflow */
  pos_max = 0;                                       move16();
  bits = 0;                                          move16();
  FOR( i=0; i<Nsv; i++ )
  {
    /* find vector to quantize (criteria: nb of estimated bits) */
    pos = sort_idx[i];                             move16();
    j = shl(pos, 3);
    ptr = xriq + j;

    /* compute multi-rate index of rounded subvector (nq,I,kv[]) */
    RE8_Cod( ptr, &nq[pos], &I[pos], &kv[j] );

    IF (nq[pos] > 0)
    {
      j = s_max(pos_max, pos);

      /* check for overflow and compute number of bits-1 (n) */
      overflow = Calc_bits(nq[pos]);

      /* check for overflow and compute number of bits-1 (n) */
      IF (sub(add(add(bits, overflow), j), nb_bits) <= 0)
      {
        bits = add(overflow, bits);
        /* update index of the last described subvector */
        pos_max = j;                           move16();
      }
      ELSE
      {
        /* if budget overflow */
        zero16(8, ptr);
        nq[pos] = 0; /* force Q0 */            move16();
      }
    }
  }

  /* write indexes to the bitstream */
  /* ============================== */
  bits = nb_bits;                                    move16();
  overflow = 0;                                      move16();
  FOR( i=0; i<Nsv; i++ )
  {
    /* 5*nq[i]-1 */
    j = Calc_bits(nq[i]);
    if (sub(j, bits) == 0) /* check the overflow */
    {
      overflow = 1;                              move16();
    }

    IF( sub(bits, 8) > 0 )
    {
      /* write the unary code for nq[i] */
      tmp16 = sub(nq[i], 1);

      FOR (j = tmp16; j > 0; j--)		
      {
        *(*pBst)++ = ITU_G192_BIT_1;			move16();	
      }
      /* s_max used because we do not sub for tmp16 < 0 */
      bits = sub(bits, s_max(0, tmp16));

      IF( !overflow )
      {
        /* write the stop bit */
        *(*pBst) = ITU_G192_BIT_0;				move16();	
        (*pBst)++;								
        bits = sub(bits, 1);
      }
      /* write codebook indices (rank I and event. Voronoi index kv) */
      IF( nq[i] != 0 )    /* for Q0 nothing to write */
      {
        IF( sub(nq[i], 5) < 0 )    /* Q2, Q3, Q4 */
        {
          tmp16 = shl(nq[i], 2);
          pos = 0;                            move16();
        }
        ELSE            /* for Q3/Q4 + Voronoi extensions r=1,2 */
        {
          j = 1; move16();
          if(s_and(nq[i], 1) == 0 )    
          {
            j = add(j,1);
          }
          pos = sub(shr(nq[i], 1), j);  /* Voronoi order determination */
          tmp16 = shl(add(j, 2), 2);
        }

        PushBitLong(I[i], pBst, tmp16 );
        bits = sub(bits, tmp16);

        IF( pos > 0 )
        {
          tmp16 = shl(i, 3);
          FOR( j=0; j<8; j++ )
          {
            /* kv[tmp16++] not counted because could be a pointer */
            PushBitLong( (Word32) kv[tmp16++], pBst, pos );
          }
          bits = sub(bits, shl(pos, 3));
        }
      }
    }
  }

  /*
  * BIT PACKING/UNPACKING IS USUALLY NOT INSTRUMENTED
  */
  FOR( i=0; i < bits; i++ )   /* fill the rest of the bitstream */	
  {
    *(*pBst) = ITU_G192_BIT_0;	move16();	
    (*pBst)++;					
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

  return (bits);
}

/*-------------------------------------------------------------------*
* AVQ_Cod
*
* SPLIT ALGEBRAIC VECTOR QUANTIZER BASED ON RE8 LATTICE
* NOTE: a mitsmatch can occurs in some subvectors between the encoder
*       and decoder, because the encoder use a bit-rate estimator to set
*       the TCX global gain - this estimator is many times faster than the
*       call of RE8_idx() for bits calculation.
*-------------------------------------------------------------------*/
void AVQ_Cod(
             Word16 *xri,    /* i  : vector to quantize                      */
             Word16 *xriq,   /* o  : quantized normalized vector (assuming the bit budget is enough) */
             Word16 NB_BITS, /* i  : number of allocated bits                */
             Word16 Nsv      /* i  : number of subvectors (lg=Nsv*8)         */
             )
{
  Word16 i, l, iter;

  Word16 gain_inv, tmp, nbits, nbits_max, fac, offset;
  Word16 ebits[NSV_MAX], e_ebits, f_ebits, e_tmp,f_tmp, tmp16;
  Word32 ener, Ltmp, Lgain, x1[8];
  Word16 *ptr0, *ptr1;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (((14 + NSV_MAX) * SIZE_Word16) + (11 * SIZE_Word32) + (2 * SIZE_Ptr)), "dummy");
#endif
  /*****************************/

  /* find energy of each subvector in log domain (scaled for bits estimation) */
  ptr0 = xri;
  FOR (l=0; l<Nsv; l++)
  {
    ener = L_add(4, L_mac_Array(8, ptr0, ptr0));
    ptr0 += 8;
    /* estimated bit consumption when gain=1 */
    f_ebits = Log2_norm_lc(norm_l_L_shl(&e_ebits, ener));
    e_ebits = sub(30-2, e_ebits);            /* -2 = *0.25 */
    Ltmp = Mpy_32_16(e_ebits, f_ebits, 40);  /* 40 = 5*8*/
    ebits[l] = extract_l(Ltmp);   /*Q4*/
    move16();
  }

  /*----------------------------------------------------------------*
  * subvector energy worst case:
  * - typically, it's a tone with maximum of amplitude (RMS=23170).
  * - fft length max = 1024 (N/2 is 512)
  * log10(energy) = log10(23710*23710*1024*(N/2)) = 14.45
  * ebits --> 5.0*FAC_LOG2*14.45 = 240 bits
  *----------------------------------------------------------------*/

  /* estimate gain according to number of bits allowed */
  /* start at the middle (offset range = 0 to 255.75) Q6 */
  fac = 2048;                                        move16();
  offset = 0;                                        move16();
  Ltmp = L_mult(31130, sub(NB_BITS, Nsv));  /* (1810 - 8 - 1152/8)*.95*/
  nbits_max = round_fx_L_shl(Ltmp, 4);

  /* tree search with 10 iterations : offset with step of 0.25 bits (0.3 dB) */
  FOR (iter=0; iter<10; iter++)
  {
    offset = add(fac, offset);
    /* calculate the required number of bits */
    nbits = 0;                                     move16();
    FOR (l=0; l<Nsv; l++)
    {
      tmp = sub(ebits[l], offset);
      tmp = s_max(tmp, 0);
      nbits = add(tmp, nbits);
    }
    /* decrease gain when no overflow occurs */
    if (sub(nbits, nbits_max) <= 0)
    {
      offset = sub(offset, fac);
    }
    fac = mult(fac, 16384);
  }

  Ltmp = L_shr(L_mult(offset, 13107), 6); /* offset((2^21)/160 */

  /* estimated gain (when offset=0, estimated gain=1) */
  f_tmp = L_Extract_lc(Ltmp, &e_tmp);
  tmp16 = extract_l(Pow2(14, f_tmp));
  Lgain = L_shl(tmp16, e_tmp);
  /* gain_inv = 1.0f / gain */
  e_tmp = norm_l(Lgain);
  tmp16 = extract_h_L_shl(Lgain, e_tmp);
  e_tmp = sub(31-14, e_tmp);
  gain_inv = div_s(16384, tmp16);
  e_tmp = sub(0, e_tmp);

  /* quantize all subvector using estimated gain */
  ptr0 = xri;
  ptr1 = xriq;

  FOR (l=0; l<Nsv; l++)
  {
    FOR (i=0; i<8; i++)
    {
      x1[i] = L_shl(L_mult(*ptr0, gain_inv), e_tmp);     move32();
      ptr0++;
    }

    RE8_PPV(x1, ptr1);
    ptr1+=8;
  }

  /* round bit allocations and save */
  array_oper(Nsv, 3, ebits, ptr1, &shl);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
}

/*------------------------------------------------------------------------
* RE8_cod:
*
* MULTI-RATE INDEXING OF A POINT y in THE LATTICE RE8 (INDEX COMPUTATION)
* note: the index I is defined as a 32-bit word, but only
*       16 bits are required (long can be replaced by unsigned integer)
*--------------------------------------------------------------------------*/
static void RE8_Cod(
                    Word16 x[],  /* i  : point in RE8 (8-dimensional integer vector)                         */
                    Word16 *n,   /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max})    */
                    UWord16 *I,   /* o  : index of c (pointer to unsigned 16-bit word)                        */
                    Word16 k[]   /* o  : index of v (8-dimensional vector of binary indices) = Voronoi index */
)
{
  Word16 ka, c[8];

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (9 * SIZE_Word16), "dummy");
#endif
  /*****************************/

  /*----------------------------------------------------------------------
  * decompose x as x = 2^r c + v, where r is an integer >=0, c is an element
  *  of Q0, Q2, Q3 or Q4, and v is an element of a Voronoi code in RE8
  *  (if r=0, x=c)
  *  this decomposition produces as a side-product the index k[] of v
  *  and the identifier ka of the absolute leader related to c
  *
  *  the index of y is split into 2 parts :
  *  - the index I of c
  *  - the index k[] of v
  ----------------------------------------------------------------------*/
  RE8_Vor(x, n, k, c, &ka);
  /* compute the index I (only if c is in Q2, Q3 or Q4) */
  if (*n > 0)
  {
    re8_compute_base_index(c, ka, I);
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/* calculate subband energy */
void loadSubbandEnergy (Word16 cod_Mode, Word16 *sEnv_BWE, Word16 *sFenv_BWE)
{
  Word16 *ptr, i;
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (SIZE_Word16 + SIZE_Ptr), "dummy");
#endif

  IF (sub ((Word16) cod_Mode, TRANSIENT) == 0)
  {
    ptr = sFenv_BWE;
    FOR (i=0; i<N_SV_2; i++)
    {
      *ptr++= sEnv_BWE[i]; move16(); /* Q(sYfb_Q=scoef_SWBQ+index_g_5bit) */
      *ptr++ = sEnv_BWE[i]; move16(); /* Q(sYfb_Q=scoef_SWBQ+index_g_5bit) */
    }
  }
  ELSE
  {
    mov16(N_SV, sEnv_BWE, sFenv_BWE);
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  return;
}
