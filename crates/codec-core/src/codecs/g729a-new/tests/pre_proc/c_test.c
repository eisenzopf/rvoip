/* ITU-T G.729 Software Package Release 2 (November 2006) */
/*
   ITU-T G.729A Speech Coder    ANSI-C Source Code
   Version 1.1    Last modified: September 1996
   Copyright (c) 1996,
   AT&T, France Telecom, NTT, Universite de Sherbrooke
   All rights reserved.
*/

#include <stdio.h>
#include <stdlib.h>

/*-----------------------------------------------------------------*
 *                        TYPEDEF.H                                *
 *-----------------------------------------------------------------*/
#if defined(__BORLANDC__) || defined (__WATCOMC__) || defined(_MSC_VER) || defined(__ZTC__) || defined(__HIGHC__) || defined(_TURBOC_)
typedef  long  int   Word32   ;
typedef  short int   Word16   ;
typedef  short int   Flag  ;
#elif defined( __sun)
typedef short  Word16;
typedef long  Word32;
typedef int   Flag;
#elif defined(__unix__) || defined(__unix) || defined(__APPLE__)
typedef short Word16;
typedef int   Word32;
typedef int   Flag;
#elif defined(VMS) || defined(__VMS)
typedef short  Word16;
typedef long  Word32;
typedef int   Flag;
#else
#error  COMPILER NOT TESTED typedef.h needs to be updated, see readme
#endif

/*-----------------------------------------------------------------*
 *                        BASIC_OP.H                               *
 *-----------------------------------------------------------------*/
extern Flag Overflow;
extern Flag Carry;

#define MAX_32 (Word32)0x7fffffffL
#define MIN_32 (Word32)0x80000000L

#define MAX_16 (Word16)0x7fff
#define MIN_16 (Word16)0x8000

Word16 sature(Word32 L_var1);
Word16 add(Word16 var1, Word16 var2);
Word16 sub(Word16 var1, Word16 var2);
Word16 abs_s(Word16 var1);
Word16 shl(Word16 var1, Word16 var2);
Word16 shr(Word16 var1, Word16 var2);
Word16 mult(Word16 var1, Word16 var2);
Word32 L_mult(Word16 var1, Word16 var2);
Word16 negate(Word16 var1);
Word16 extract_h(Word32 L_var1);
Word16 extract_l(Word32 L_var1);
Word16 g729a_round(Word32 L_var1);
Word32 L_mac(Word32 L_var3, Word16 var1, Word16 var2);
Word32 L_msu(Word32 L_var3, Word16 var1, Word16 var2);
Word32 L_add(Word32 L_var1, Word32 L_var2);
Word32 L_sub(Word32 L_var1, Word32 L_var2);
Word32 L_shl(Word32 L_var1, Word16 var2);
Word32 L_shr(Word32 L_var1, Word16 var2);

/*-----------------------------------------------------------------*
 *                        OPER_32B.H                               *
 *-----------------------------------------------------------------*/
void   L_Extract(Word32 L_32, Word16 *hi, Word16 *lo);
Word32 Mpy_32_16(Word16 hi, Word16 lo, Word16 n);

/*-----------------------------------------------------------------*
 *                        BASIC_OP.C                               *
 *-----------------------------------------------------------------*/
Flag Overflow = 0;
Flag Carry = 0;

Word16 sature(Word32 L_var1) {
    Word16 var_out;
    if (L_var1 > 0X00007fffL) {
        Overflow = 1;
        var_out = MAX_16;
    } else if (L_var1 < (Word32)0xffff8000L) {
        Overflow = 1;
        var_out = MIN_16;
    } else {
        Overflow = 0;
        var_out = extract_l(L_var1);
    }
    return(var_out);
}

Word16 add(Word16 var1, Word16 var2) {
    Word16 var_out;
    Word32 L_somme;
    L_somme = (Word32) var1 + var2;
    var_out = sature(L_somme);
    return(var_out);
}

Word16 sub(Word16 var1, Word16 var2) {
    Word16 var_out;
    Word32 L_diff;
    L_diff = (Word32) var1 - var2;
    var_out = sature(L_diff);
    return(var_out);
}

Word16 shl(Word16 var1, Word16 var2) {
    Word16 var_out;
    Word32 resultat;
    if (var2 < 0) {
        var_out = shr(var1, -var2);
    } else {
        resultat = (Word32) var1 * ((Word32) 1 << var2);
        if ((var2 > 15 && var1 != 0) || (resultat != (Word32)((Word16) resultat))) {
            Overflow = 1;
            var_out = (var1 > 0) ? MAX_16 : MIN_16;
        }
        else {
            var_out = extract_l(resultat);
        }
    }
    return(var_out);
}

Word16 shr(Word16 var1, Word16 var2) {
    Word16 var_out;
    if (var2 < 0) {
        var_out = shl(var1, -var2);
    } else {
        if (var2 >= 15) {
            var_out = (var1 < 0) ? (Word16)(-1) : (Word16)0;
        }
        else {
            if (var1 < 0) var_out = ~((~var1) >> var2);
            else var_out = var1 >> var2;
        }
    }
    return(var_out);
}

Word16 mult(Word16 var1, Word16 var2) {
    Word16 var_out;
    Word32 L_produit;
    L_produit = (Word32)var1 * (Word32)var2;
    L_produit = (L_produit & (Word32)0xffff8000L) >> 15;
    if (L_produit & (Word32)0x00010000L) L_produit = L_produit | (Word32)0xffff0000L;
    var_out = sature(L_produit);
    return(var_out);
}

Word32 L_mult(Word16 var1, Word16 var2) {
    Word32 L_var_out;
    L_var_out = (Word32)var1 * (Word32)var2;
    if (L_var_out != (Word32)0x40000000L) {
        L_var_out *= 2;
    } else {
        Overflow = 1;
        L_var_out = MAX_32;
    }
    return(L_var_out);
}

Word16 extract_h(Word32 L_var1) {
    return (Word16)(L_var1 >> 16);
}

Word16 extract_l(Word32 L_var1) {
    return (Word16)L_var1;
}

Word32 L_add(Word32 L_var1, Word32 L_var2) {
    Word32 L_var_out;
    L_var_out = L_var1 + L_var2;
    if (((L_var1 ^ L_var2) & MIN_32) == 0) {
        if ((L_var_out ^ L_var1) & MIN_32) {
            L_var_out = (L_var1 < 0) ? MIN_32 : MAX_32;
            Overflow = 1;
        }
    }
    return(L_var_out);
}

Word32 L_sub(Word32 L_var1, Word32 L_var2) {
    Word32 L_var_out;
    L_var_out = L_var1 - L_var2;
    if (((L_var1 ^ L_var2) & MIN_32) != 0) {
        if ((L_var_out ^ L_var1) & MIN_32) {
            L_var_out = (L_var1 < 0L) ? MIN_32 : MAX_32;
            Overflow = 1;
        }
    }
    return(L_var_out);
}

Word32 L_shl(Word32 L_var1, Word16 var2) {
    Word32 L_var_out = 0L;
    if (var2 <= 0) {
        L_var_out = L_shr(L_var1, -var2);
    } else {
        for (; var2 > 0; var2--) {
            if (L_var1 > (Word32)0X3fffffffL) {
                Overflow = 1;
                L_var_out = MAX_32;
                break;
            } else {
                if (L_var1 < (Word32)0xc0000000L) {
                    Overflow = 1;
                    L_var_out = MIN_32;
                    break;
                }
            }
            L_var1 *= 2;
            L_var_out = L_var1;
        }
    }
    return(L_var_out);
}

Word32 L_shr(Word32 L_var1, Word16 var2) {
    Word32 L_var_out;
    if (var2 < 0) {
        L_var_out = L_shl(L_var1, -var2);
    } else {
        if (var2 >= 31) {
            L_var_out = (L_var1 < 0L) ? -1 : 0;
        }
        else {
            if (L_var1 < 0) L_var_out = ~((~L_var1) >> var2);
            else L_var_out = L_var1 >> var2;
        }
    }
    return(L_var_out);
}

Word32 L_mac(Word32 L_var3, Word16 var1, Word16 var2) {
    Word32 L_produit;
    L_produit = L_mult(var1, var2);
    return L_add(L_var3, L_produit);
}

Word32 L_msu(Word32 L_var3, Word16 var1, Word16 var2) {
    Word32 L_produit;
    L_produit = L_mult(var1, var2);
    return L_sub(L_var3, L_produit);
}

Word16 g729a_round(Word32 L_var1) {
    Word32 L_arrondi;
    L_arrondi = L_add(L_var1, (Word32)0x00008000);
    return extract_h(L_arrondi);
}

/*-----------------------------------------------------------------*
 *                        OPER_32B.C                               *
 *-----------------------------------------------------------------*/
void L_Extract(Word32 L_32, Word16 *hi, Word16 *lo) {
    *hi = extract_h(L_32);
    *lo = extract_l(L_msu(L_shr(L_32, 1), *hi, 16384));
}

Word32 Mpy_32_16(Word16 hi, Word16 lo, Word16 n) {
    Word32 L_32;
    L_32 = L_mult(hi, n);
    L_32 = L_mac(L_32, mult(lo, n), 1);
    return(L_32);
}

/*-----------------------------------------------------------------*
 *                        TAB_LD8A.C                               *
 *-----------------------------------------------------------------*/
Word16 b140[3] = {1899, -3798, 1899};      /* 1/2 in Q12 */
Word16 a140[3] = {4096, 7807, -3733};      /* Q12 */

/*-----------------------------------------------------------------*
 *                        PRE_PROC.C                               *
 *-----------------------------------------------------------------*/
static Word16 y2_hi, y2_lo, y1_hi, y1_lo, x0, x1;

void Init_Pre_Process(void)
{
  y2_hi = 0;
  y2_lo = 0;
  y1_hi = 0;
  y1_lo = 0;
  x0   = 0;
  x1   = 0;
}

void Pre_Process(
  Word16 signal[],
  Word16 lg)
{
  Word16 i, x2;
  Word32 L_tmp;

  for(i=0; i<lg; i++)
  {
     x2 = x1;
     x1 = x0;
     x0 = signal[i];

     L_tmp     = Mpy_32_16(y1_hi, y1_lo, a140[1]);
     L_tmp     = L_add(L_tmp, Mpy_32_16(y2_hi, y2_lo, a140[2]));
     L_tmp     = L_mac(L_tmp, x0, b140[0]);
     L_tmp     = L_mac(L_tmp, x1, b140[1]);
     L_tmp     = L_mac(L_tmp, x2, b140[2]);
     L_tmp     = L_shl(L_tmp, 3);
     signal[i] = g729a_round(L_tmp);

     y2_hi = y1_hi;
     y2_lo = y1_lo;
     L_Extract(L_tmp, &y1_hi, &y1_lo);
  }
  return;
}

/*-----------------------------------------------------------------*
 *                        MAIN                                     *
 *-----------------------------------------------------------------*/
int main(void) {
    Word16 signal[80];
    int i;

    for (i = 0; i < 80; i++) {
        signal[i] = 8192;
    }

    Init_Pre_Process();
    Pre_Process(signal, 80);

    printf("c_function_name,c_output\n");
    for (i = 0; i < 80; i++) {
        printf("pre_proc,%d\n", signal[i]);
    }

    return 0;
}
