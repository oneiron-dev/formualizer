//! Financial functions
//! Functions implemented: PMT, PV, FV, NPV, NPER, RATE, IPMT, PPMT, SLN, SYD, DB, DDB,
//! VDB, AMORLINC, AMORDEGRC, XNPV, XIRR, DOLLARDE, DOLLARFR, FVSCHEDULE, ACCRINT,
//! ACCRINTM, PRICE, YIELD, DURATION, MDURATION, TBILLEQ, TBILLPRICE, TBILLYIELD, ISPMT,
//! PDURATION, COUPDAYBS, COUPDAYS, COUPDAYSNC, COUPNCD, COUPNUM, COUPPCD, DISC, INTRATE,
//! RECEIVED, PRICEDISC, YIELDDISC, PRICEMAT, YIELDMAT, ODDFPRICE, ODDFYIELD, ODDLPRICE,
//! ODDLYIELD

mod bonds;
mod coupon;
mod depreciation;
mod discount;
mod odd;
mod tvm;

pub fn register_builtins() {
    bonds::register_builtins();
    coupon::register_builtins();
    discount::register_builtins();
    odd::register_builtins();
    tvm::register_builtins();
    depreciation::register_builtins();
}
