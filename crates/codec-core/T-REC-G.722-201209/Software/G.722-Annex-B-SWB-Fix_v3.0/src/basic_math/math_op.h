/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------*
 *                         MATH_OP.H	                                    *
 *--------------------------------------------------------------------------*
 *       Mathematical operations                                            *
 *--------------------------------------------------------------------------*/
#include "oper_32b.h"
#include "log2.h"

Word32 Isqrt_lc(
     Word32 frac,       /* (i/o) Q31: normalized value (1.0 < frac <= 0.5) */
     Word16 * exp       /* (i/o)    : exponent (value = frac x 2^exponent) */
);

Word32 Pow2(            /* (o) Q0  : result       (range: 0<=val<=0x7fffffff) */
     Word16 exponant,   /* (i) Q0  : Integer part.      (range: 0<=val<=30)   */
     Word16 fraction    /* (i) Q15 : Fractionnal part.  (range: 0.0<=val<1.0) */
);

Word32 L_Frac_sqrtQ31(  /* o  : Square root if input */
    const Word32 x      /* i  : Input                */
);

Word16 L_sqrt(Word32 Num);
void Log2(Word32 L_x,           /* (i) Q0 : input value                                 */
          Word16 * exponent,    /* (o) Q0 : Integer part of Log2.   (range: 0<=val<=30) */
          Word16 * fraction     /* (o) Q15: Fractional  part of Log2. (range: 0<=val<1) */
    );

